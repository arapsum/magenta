use super::*;
use magenta_core::FinishReason;

#[test]
fn stream_events_keep_text_and_completion_order() {
    let delta: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_text.delta",
        "delta": "hello"
    }))
    .expect("delta should deserialize");
    assert!(matches!(
        OpenAiProvider::stream_event(&delta),
        Ok(Some(StreamOutput::Text(value))) if value == "hello"
    ));

    let completed: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.completed",
        "response": {"usage": {"input_tokens": 1, "output_tokens": 2}}
    }))
    .expect("completion should deserialize");
    assert!(matches!(
        OpenAiProvider::stream_event(&completed),
        Ok(Some(StreamOutput::Completed(GenerationOutcome {
            finish_reason: FinishReason::Stop,
            ..
        })))
    ));
}

#[test]
fn status_classification_preserves_auth_and_service_failures() {
    assert_eq!(
        classify_status(401),
        ProviderErrorKind::AuthenticationRequired
    );
    assert_eq!(classify_status(429), ProviderErrorKind::RateLimited);
    assert_eq!(classify_status(503), ProviderErrorKind::ServiceUnavailable);
}

#[test]
fn codex_headers_use_the_oh_my_pi_compatibility_values() {
    let headers = OpenAiProvider::headers(
        "Bearer token",
        Some("account-123"),
        "application/json",
        None,
    );

    assert!(headers.contains(&("openai-beta", "responses=experimental")));
    assert!(headers.contains(&("originator", "omp")));
    assert!(headers.contains(&("version", CLIENT_VERSION)));
    assert!(headers.contains(&("chatgpt-account-id", "account-123")));
}
