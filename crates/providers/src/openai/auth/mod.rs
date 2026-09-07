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
mod claims;
#[cfg(test)]
mod tests;
use claims::{
    auth_error, authorization_url, form_body, now_ms, oauth_error_message, random_urlsafe,
    record_from_token_response, redirect_uri,
};

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

        let refresh_window_ms = REFRESH_WINDOW.as_secs().saturating_mul(1_000);
        expires_at_ms <= now_ms().saturating_add(refresh_window_ms)
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
        concat!(
            "<!doctype html><html><body ",
            "style=\"font-family:sans-serif;background:#090e0f;color:#d9f9ff\">",
            "<p>{}</p></body></html>"
        ),
        message
    );
    let response = format!(
        concat!(
            "HTTP/1.1 {}\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "Content-Length: {}\r\n",
            "Connection: close\r\n",
            "\r\n",
            "{}"
        ),
        status,
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| OpenAiAuthError::CallbackRejected(error.to_string()))
}
