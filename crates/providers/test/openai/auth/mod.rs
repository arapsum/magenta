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
