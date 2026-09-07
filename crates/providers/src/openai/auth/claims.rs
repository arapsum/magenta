use super::*;

pub(super) fn authorization_url(state: &str, verifier: &str) -> Result<String, OpenAiAuthError> {
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

pub(super) fn redirect_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}")
}

pub(super) fn form_body<const N: usize>(pairs: [(&str, &str); N]) -> Vec<u8> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish().into_bytes()
}

pub(super) fn random_urlsafe(byte_count: usize) -> String {
    let mut bytes = vec![0; byte_count];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn record_from_token_response(
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

pub(super) fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn account_id_claim(claims: &serde_json::Value) -> Option<String> {
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

pub(super) fn plan_claim(claims: &serde_json::Value) -> Option<String> {
    claim_string(claims, "chatgpt_plan_type")
        .or_else(|| nested_claim(claims, JWT_AUTH_CLAIM, "chatgpt_plan_type"))
}

pub(super) fn email_claim(claims: &serde_json::Value) -> Option<String> {
    nested_claim(claims, JWT_PROFILE_CLAIM, "email")
        .or_else(|| claim_string(claims, "email"))
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
}

pub(super) fn name_claim(claims: &serde_json::Value) -> Option<String> {
    nested_claim(claims, JWT_PROFILE_CLAIM, "name")
        .or_else(|| claim_string(claims, "name"))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

pub(super) fn nested_claim(
    claims: &serde_json::Value,
    namespace: &str,
    key: &str,
) -> Option<String> {
    claims
        .get(namespace)
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

pub(super) fn claim_string(claims: &serde_json::Value, key: &str) -> Option<String> {
    claims
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

pub(super) fn oauth_error_message(status: u16, body: &[u8]) -> String {
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

pub(super) fn auth_error(kind: ProviderErrorKind, error: OpenAiAuthError) -> ProviderError {
    ProviderError::with_kind(openai_provider(), kind, error)
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration
                .as_secs()
                .saturating_mul(1_000)
                .saturating_add(u64::from(duration.subsec_millis()))
        })
}
