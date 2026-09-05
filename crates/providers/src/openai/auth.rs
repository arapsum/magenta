use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_lock::Mutex;
use async_net::{TcpListener, TcpStream};
use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use futures_util::{
    FutureExt as _,
    future::{Either, select},
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
};
use http_client::{HttpClient, Method};
use magenta_core::{
    AuthenticationFuture, AuthorizationSession, ProviderAccount, ProviderAuthenticator,
    ProviderError, ProviderErrorKind, ProviderId,
};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::http;

pub fn openai_provider() -> ProviderId {
    ProviderId::new("openai")
}

const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CALLBACK_PATH: &str = "/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const OAUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const JWT_AUTH_CLAIM: &str = "https://api.openai.com/auth";
const JWT_PROFILE_CLAIM: &str = "https://api.openai.com/profile";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const REFRESH_WINDOW: Duration = Duration::from_secs(300);
const KEYRING_SERVICE: &str = "dev.magenta.desktop";
const KEYRING_USER: &str = "openai-codex";
const MAX_ERROR_BODY: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
enum OpenAiAuthError {
    #[error("the OpenAI account is not signed in")]
    AuthenticationRequired,
    #[error("could not open a local OAuth callback listener: {0}")]
    CallbackListener(String),
    #[error("the OAuth callback timed out")]
    CallbackTimeout,
    #[error("the OAuth callback was rejected: {0}")]
    CallbackRejected(String),
    #[error("the OAuth token exchange failed: {0}")]
    TokenExchange(String),
    #[error("the OAuth response did not contain an access token")]
    MissingAccessToken,
    #[error("the saved OpenAI credentials are invalid: {0}")]
    StoredCredentials(String),
    #[error("the secure credential store failed: {0}")]
    CredentialStore(String),
    #[error("could not build the OpenAI authorization URL: {0}")]
    AuthorizationUrl(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CredentialRecord {
    access_token: String,
    refresh_token: Option<String>,
    account_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    email: Option<String>,
    plan: Option<String>,
    expires_at_ms: Option<u64>,
}

impl CredentialRecord {
    fn account(&self) -> ProviderAccount {
        ProviderAccount {
            provider: openai_provider(),
            name: self.name.clone(),
            email: self.email.clone(),
            plan: self.plan.clone(),
        }
    }

    fn needs_refresh(&self) -> bool {
        let Some(expires_at_ms) = self.expires_at_ms else {
            return false;
        };

        expires_at_ms <= now_ms().saturating_add(REFRESH_WINDOW.as_secs().saturating_mul(1_000))
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

trait CredentialStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, String>;
    fn save(&self, value: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

#[derive(Debug, Default)]
struct KeyringCredentialStore;

impl KeyringCredentialStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())
    }
}

impl CredentialStore for KeyringCredentialStore {
    fn load(&self) -> Result<Option<String>, String> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn save(&self, value: &str) -> Result<(), String> {
        Self::entry()?
            .set_password(value)
            .map_err(|error| error.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct OpenAiAuth {
    client: Arc<dyn HttpClient>,
    store: Arc<dyn CredentialStore>,
    state: Arc<Mutex<Option<CredentialRecord>>>,
    refresh_lock: Arc<Mutex<()>>,
}

impl OpenAiAuth {
    pub(crate) fn new(client: Arc<dyn HttpClient>) -> Self {
        Self::with_store(client, Arc::new(KeyringCredentialStore))
    }

    fn with_store(client: Arc<dyn HttpClient>, store: Arc<dyn CredentialStore>) -> Self {
        Self {
            client,
            store,
            state: Arc::new(Mutex::new(None)),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) async fn restore_inner(&self) -> Result<Option<ProviderAccount>, ProviderError> {
        let store = Arc::clone(&self.store);
        let value = smol::unblock(move || store.load()).await.map_err(|error| {
            auth_error(
                ProviderErrorKind::Other,
                OpenAiAuthError::CredentialStore(error),
            )
        })?;
        let Some(value) = value else {
            return Ok(None);
        };

        let record = serde_json::from_str::<CredentialRecord>(&value).map_err(|error| {
            auth_error(
                ProviderErrorKind::Other,
                OpenAiAuthError::StoredCredentials(error.to_string()),
            )
        })?;
        if record.access_token.is_empty() {
            return Err(auth_error(
                ProviderErrorKind::Other,
                OpenAiAuthError::StoredCredentials("access token is empty".to_owned()),
            ));
        }

        let record = record_from_token_response(
            TokenResponse {
                access_token: record.access_token.clone(),
                refresh_token: record.refresh_token.clone(),
                expires_in: None,
                id_token: None,
            },
            Some(&record),
        )
        .map_err(|error| auth_error(ProviderErrorKind::Other, error))?;

        let record = if record.needs_refresh() {
            self.refresh_record(record)
                .await
                .map_err(|error| auth_error(ProviderErrorKind::AuthenticationRequired, error))?
        } else {
            record
        };
        let account = record.account();
        *self.state.lock().await = Some(record);
        Ok(Some(account))
    }

    pub(crate) async fn begin_login_inner(&self) -> Result<AuthorizationSession, ProviderError> {
        let listener = bind_callback_listener()
            .await
            .map_err(|error| auth_error(ProviderErrorKind::Other, error))?;
        let verifier = random_urlsafe(32);
        let state = random_urlsafe(24);
        let authorization_url = authorization_url(&state, &verifier)
            .map_err(|error| auth_error(ProviderErrorKind::Other, error))?;

        let client = Arc::clone(&self.client);
        let store = Arc::clone(&self.store);
        let state_slot = Arc::clone(&self.state);
        let completion = async move {
            let auth = Self {
                client,
                store,
                state: state_slot,
                refresh_lock: Arc::new(Mutex::new(())),
            };
            let code = wait_for_callback(listener, &state)
                .await
                .map_err(|error| auth_error(ProviderErrorKind::AuthenticationRequired, error))?;
            let token = auth
                .exchange_code(&code, &verifier)
                .await
                .map_err(|error| auth_error(ProviderErrorKind::AuthenticationRequired, error))?;
            let record = record_from_token_response(token, None)
                .map_err(|error| auth_error(ProviderErrorKind::AuthenticationRequired, error))?;
            auth.persist(&record)
                .await
                .map_err(|error| auth_error(ProviderErrorKind::Other, error))?;
            let account = record.account();
            *auth.state.lock().await = Some(record);
            Ok(account)
        };

        Ok(AuthorizationSession {
            authorization_url,
            completion: Box::pin(completion),
        })
    }

    pub(crate) async fn access_token(&self) -> Result<String, ProviderError> {
        let record = self.state.lock().await.clone().ok_or_else(|| {
            auth_error(
                ProviderErrorKind::AuthenticationRequired,
                OpenAiAuthError::AuthenticationRequired,
            )
        })?;

        if !record.needs_refresh() {
            return Ok(record.access_token);
        }

        self.refresh_record(record)
            .await
            .map(|record| record.access_token)
            .map_err(|error| auth_error(ProviderErrorKind::AuthenticationRequired, error))
    }

    pub(crate) async fn account_id(&self) -> Option<String> {
        self.state
            .lock()
            .await
            .as_ref()
            .and_then(|record| record.account_id.clone())
    }

    pub(crate) async fn force_refresh(
        &self,
        failed_access_token: &str,
    ) -> Result<String, ProviderError> {
        let _guard = self.refresh_lock.lock().await;
        let current = self.state.lock().await.clone().ok_or_else(|| {
            auth_error(
                ProviderErrorKind::AuthenticationRequired,
                OpenAiAuthError::AuthenticationRequired,
            )
        })?;
        if current.access_token != failed_access_token {
            return Ok(current.access_token);
        }

        self.refresh_record_locked(current)
            .await
            .map(|record| record.access_token)
            .map_err(|error| auth_error(ProviderErrorKind::AuthenticationRequired, error))
    }

    async fn refresh_record(
        &self,
        stale: CredentialRecord,
    ) -> Result<CredentialRecord, OpenAiAuthError> {
        let _guard = self.refresh_lock.lock().await;
        let current = self.state.lock().await.clone();
        if let Some(current) = current
            && current.access_token != stale.access_token
            && !current.needs_refresh()
        {
            return Ok(current);
        }
        self.refresh_record_locked(stale).await
    }

    async fn refresh_record_locked(
        &self,
        stale: CredentialRecord,
    ) -> Result<CredentialRecord, OpenAiAuthError> {
        let Some(refresh_token) = stale.refresh_token.clone() else {
            return Err(OpenAiAuthError::AuthenticationRequired);
        };
        let token = self.exchange_refresh_token(&refresh_token).await?;
        let record = record_from_token_response(token, Some(&stale))?;
        self.persist(&record).await?;
        *self.state.lock().await = Some(record.clone());
        Ok(record)
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OpenAiAuthError> {
        let body = form_body([
            ("grant_type", "authorization_code"),
            ("client_id", OAUTH_CLIENT_ID),
            ("code", code),
            ("redirect_uri", redirect_uri().as_str()),
            ("code_verifier", verifier),
        ]);
        self.token_request(body).await
    }

    async fn exchange_refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<TokenResponse, OpenAiAuthError> {
        let body = form_body([
            ("grant_type", "refresh_token"),
            ("client_id", OAUTH_CLIENT_ID),
            ("refresh_token", refresh_token),
        ]);
        self.token_request(body).await
    }

    async fn token_request(&self, body: Vec<u8>) -> Result<TokenResponse, OpenAiAuthError> {
        let request = http::request(
            Method::POST,
            TOKEN_URL,
            &[
                ("Accept", "application/json"),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
            body,
            Duration::from_secs(30),
        )
        .map_err(|error| OpenAiAuthError::TokenExchange(error.to_string()))?;
        let mut response = http::send(self.client.as_ref(), request)
            .await
            .map_err(OpenAiAuthError::TokenExchange)?;
        let status = response.status();
        let body = http::read_limited(response.body_mut(), MAX_ERROR_BODY)
            .await
            .map_err(|error| OpenAiAuthError::TokenExchange(error.to_string()))?;
        if !status.is_success() {
            return Err(OpenAiAuthError::TokenExchange(oauth_error_message(
                status.as_u16(),
                &body,
            )));
        }

        serde_json::from_slice(&body)
            .map_err(|error| OpenAiAuthError::TokenExchange(error.to_string()))
    }

    async fn persist(&self, record: &CredentialRecord) -> Result<(), OpenAiAuthError> {
        let value = serde_json::to_string(record)
            .map_err(|error| OpenAiAuthError::StoredCredentials(error.to_string()))?;
        let store = Arc::clone(&self.store);
        smol::unblock(move || store.save(&value))
            .await
            .map_err(OpenAiAuthError::CredentialStore)?;
        Ok(())
    }

    async fn sign_out_inner(&self) -> Result<(), ProviderError> {
        *self.state.lock().await = None;
        let store = Arc::clone(&self.store);
        smol::unblock(move || store.delete())
            .await
            .map_err(|error| {
                auth_error(
                    ProviderErrorKind::Other,
                    OpenAiAuthError::CredentialStore(error),
                )
            })
    }
}

impl ProviderAuthenticator for OpenAiAuth {
    fn restore(&self) -> AuthenticationFuture<Option<ProviderAccount>> {
        let auth = self.clone();
        Box::pin(async move { auth.restore_inner().await })
    }

    fn begin_login(&self) -> AuthenticationFuture<AuthorizationSession> {
        let auth = self.clone();
        Box::pin(async move { auth.begin_login_inner().await })
    }

    fn sign_out(&self) -> AuthenticationFuture<()> {
        let auth = self.clone();
        Box::pin(async move { auth.sign_out_inner().await })
    }
}

async fn bind_callback_listener() -> Result<TcpListener, OpenAiAuthError> {
    TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|error| OpenAiAuthError::CallbackListener(error.to_string()))
}

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, OpenAiAuthError> {
    loop {
        let timeout =
            smol::Timer::after(CALLBACK_TIMEOUT).map(|_| Err(OpenAiAuthError::CallbackTimeout));
        futures_util::pin_mut!(timeout);
        let accept_listener = listener.clone();
        let accept = accept_listener.accept();
        futures_util::pin_mut!(accept);

        match select(accept, timeout).await {
            Either::Right((result, _)) => return result,
            Either::Left((result, _timeout)) => {
                let (stream, _address) =
                    result.map_err(|error| OpenAiAuthError::CallbackRejected(error.to_string()))?;
                if let Some(code) = callback_request(stream, expected_state).await? {
                    return Ok(code);
                }
            }
        }
    }
}

async fn callback_request(
    stream: TcpStream,
    expected_state: &str,
) -> Result<Option<String>, OpenAiAuthError> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|error| OpenAiAuthError::CallbackRejected(error.to_string()))?;
    let mut stream = reader.into_inner();
    let target = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| OpenAiAuthError::CallbackRejected("malformed HTTP request".to_owned()))?;
    let url = Url::parse(&format!("http://localhost{target}"))
        .map_err(|error| OpenAiAuthError::CallbackRejected(error.to_string()))?;
    if url.path() != CALLBACK_PATH {
        write_callback_response(&mut stream, "404 Not Found", "Not found.").await?;
        return Ok(None);
    }

    let query = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    if query
        .get("state")
        .is_none_or(|callback_state| callback_state != expected_state)
    {
        write_callback_response(
            &mut stream,
            "400 Bad Request",
            "The sign-in request was rejected.",
        )
        .await?;
        return Err(OpenAiAuthError::CallbackRejected(
            "state did not match".to_owned(),
        ));
    }
    if let Some(error) = query.get("error") {
        let description = query
            .get("error_description")
            .map_or("authorization was denied", |value| value.as_ref());
        write_callback_response(&mut stream, "400 Bad Request", "Sign-in was not completed.")
            .await?;
        return Err(OpenAiAuthError::CallbackRejected(format!(
            "{error}: {description}"
        )));
    }
    let code = query
        .get("code")
        .filter(|code| !code.is_empty())
        .ok_or_else(|| {
            OpenAiAuthError::CallbackRejected("authorization code is missing".to_owned())
        })?;
    write_callback_response(
        &mut stream,
        "200 OK",
        "Magenta is signed in. You can close this window.",
    )
    .await?;
    Ok(Some(code.as_ref().to_owned()))
}

async fn write_callback_response(
    stream: &mut TcpStream,
    status: &str,
    message: &str,
) -> Result<(), OpenAiAuthError> {
    let body = format!(
        "<!doctype html><html><body style=\"font-family:sans-serif;background:#090e0f;color:#d9f9ff\"><p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| OpenAiAuthError::CallbackRejected(error.to_string()))
}

fn authorization_url(state: &str, verifier: &str) -> Result<String, OpenAiAuthError> {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse(AUTHORIZE_URL)
        .map_err(|error| OpenAiAuthError::AuthorizationUrl(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", OAUTH_CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri())
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", "omp");
    Ok(url.into())
}

fn redirect_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}")
}

fn form_body<const N: usize>(pairs: [(&str, &str); N]) -> Vec<u8> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish().into_bytes()
}

fn random_urlsafe(byte_count: usize) -> String {
    let mut bytes = vec![0; byte_count];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn record_from_token_response(
    token: TokenResponse,
    previous: Option<&CredentialRecord>,
) -> Result<CredentialRecord, OpenAiAuthError> {
    if token.access_token.is_empty() {
        return Err(OpenAiAuthError::MissingAccessToken);
    }
    let id_claims = token.id_token.as_deref().and_then(jwt_claims);
    let access_claims = jwt_claims(&token.access_token);
    let account_id = id_claims
        .as_ref()
        .and_then(account_id_claim)
        .or_else(|| access_claims.as_ref().and_then(account_id_claim))
        .or_else(|| previous.and_then(|record| record.account_id.clone()));
    let email = id_claims
        .as_ref()
        .and_then(email_claim)
        .or_else(|| access_claims.as_ref().and_then(email_claim))
        .or_else(|| previous.and_then(|record| record.email.clone()));
    let name = id_claims
        .as_ref()
        .and_then(name_claim)
        .or_else(|| access_claims.as_ref().and_then(name_claim))
        .or_else(|| previous.and_then(|record| record.name.clone()));
    let plan = id_claims
        .as_ref()
        .and_then(plan_claim)
        .or_else(|| access_claims.as_ref().and_then(plan_claim))
        .or_else(|| previous.and_then(|record| record.plan.clone()));
    let expires_at_ms = token
        .expires_in
        .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1_000)))
        .or_else(|| previous.and_then(|record| record.expires_at_ms));

    Ok(CredentialRecord {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .or_else(|| previous.and_then(|record| record.refresh_token.clone())),
        account_id,
        name,
        email,
        plan,
        expires_at_ms,
    })
}

fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn account_id_claim(claims: &serde_json::Value) -> Option<String> {
    claim_string(claims, "chatgpt_account_id")
        .or_else(|| nested_claim(claims, JWT_AUTH_CLAIM, "chatgpt_account_id"))
        .or_else(|| {
            claims
                .get("organizations")
                .and_then(serde_json::Value::as_array)
                .and_then(|organizations| organizations.first())
                .and_then(|organization| organization.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn plan_claim(claims: &serde_json::Value) -> Option<String> {
    claim_string(claims, "chatgpt_plan_type")
        .or_else(|| nested_claim(claims, JWT_AUTH_CLAIM, "chatgpt_plan_type"))
}

fn email_claim(claims: &serde_json::Value) -> Option<String> {
    nested_claim(claims, JWT_PROFILE_CLAIM, "email")
        .or_else(|| claim_string(claims, "email"))
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
}

fn name_claim(claims: &serde_json::Value) -> Option<String> {
    nested_claim(claims, JWT_PROFILE_CLAIM, "name")
        .or_else(|| claim_string(claims, "name"))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

fn nested_claim(claims: &serde_json::Value, namespace: &str, key: &str) -> Option<String> {
    claims
        .get(namespace)
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn claim_string(claims: &serde_json::Value, key: &str) -> Option<String> {
    claims
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn oauth_error_message(status: u16, body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<OAuthErrorResponse>(body).ok();
    let detail = parsed
        .as_ref()
        .and_then(|error| error.error_description.as_deref())
        .or_else(|| parsed.as_ref().and_then(|error| error.error.as_deref()))
        .map(str::to_owned)
        .or_else(|| String::from_utf8(body.to_vec()).ok())
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "the authorization server rejected the request".to_owned());
    format!("HTTP {status}: {detail}")
}

fn auth_error(kind: ProviderErrorKind, error: OpenAiAuthError) -> ProviderError {
    ProviderError::with_kind(openai_provider(), kind, error)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration
                .as_secs()
                .saturating_mul(1_000)
                .saturating_add(u64::from(duration.subsec_millis()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_url_uses_pkce_and_local_callback() {
        let url = authorization_url("state", "verifier").expect("URL should build");
        let parsed = Url::parse(&url).expect("URL should parse");
        let query = parsed
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(
            query.get("client_id").map(std::convert::AsRef::as_ref),
            Some(OAUTH_CLIENT_ID)
        );
        assert_eq!(
            query.get("redirect_uri").map(std::convert::AsRef::as_ref),
            Some(redirect_uri().as_str())
        );
        assert_eq!(
            query
                .get("code_challenge_method")
                .map(std::convert::AsRef::as_ref),
            Some("S256")
        );
        assert_eq!(
            query.get("state").map(std::convert::AsRef::as_ref),
            Some("state")
        );
        assert!(
            query
                .get("code_challenge")
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[test]
    fn token_claims_preserve_account_metadata() {
        let header = URL_SAFE_NO_PAD.encode(br"{}");
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "https://api.openai.com/profile": {
                    "email": " Person@Example.com ",
                    "name": " Person Example "
                },
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "account-123",
                    "chatgpt_plan_type": "plus"
                }
            }))
            .expect("claims should serialize"),
        );
        let token = format!("{header}.{payload}.signature");
        let record = record_from_token_response(
            TokenResponse {
                access_token: token,
                refresh_token: Some("refresh".to_owned()),
                expires_in: Some(60),
                id_token: None,
            },
            None,
        )
        .expect("record should build");

        assert_eq!(record.account_id.as_deref(), Some("account-123"));
        assert_eq!(record.name.as_deref(), Some("Person Example"));
        assert_eq!(record.email.as_deref(), Some("person@example.com"));
        assert_eq!(record.plan.as_deref(), Some("plus"));
    }

    #[test]
    fn saved_credentials_without_a_name_remain_compatible() {
        let record = serde_json::from_value::<CredentialRecord>(serde_json::json!({
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "account_id": "account-123",
            "email": "person@example.com",
            "plan": "plus",
            "expires_at_ms": null
        }))
        .expect("credentials from before the display name field should load");

        assert_eq!(record.name, None);
        assert_eq!(
            record.account().email.as_deref(),
            Some("person@example.com")
        );
    }
}
