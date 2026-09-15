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
fn reasoning_summary_events_use_stable_keys_and_decode_text() {
    let added: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.reasoning_summary_part.added",
        "item_id": "rs_1",
        "summary_index": 0,
        "part": {"type": "summary_text"}
    }))
    .expect("summary part should deserialize");
    assert!(matches!(
        OpenAiProvider::stream_event(&added),
        Ok(Some(StreamOutput::ReasoningSummaryStarted { key, .. }))
            if key == "reasoning:rs_1:0"
    ));

    let delta: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.reasoning_summary_text.delta",
        "item_id": "rs_1",
        "summary_index": 0,
        "delta": "Inspecting the request"
    }))
    .expect("summary delta should deserialize");
    assert!(matches!(
        OpenAiProvider::stream_event(&delta),
        Ok(Some(StreamOutput::ReasoningSummaryDelta { key, delta }))
            if key == "reasoning:rs_1:0" && delta == "Inspecting the request"
    ));

    let done: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.reasoning_summary_text.done",
        "item_id": "rs_1",
        "summary_index": 0,
        "text": "Inspecting the request"
    }))
    .expect("summary completion should deserialize");
    assert!(matches!(
        OpenAiProvider::stream_event(&done),
        Ok(Some(StreamOutput::ReasoningSummaryCompleted { key, text }))
            if key == "reasoning:rs_1:0" && text == "Inspecting the request"
    ));
}

#[test]
fn completed_responses_fallback_to_deduplicated_reasoning_summaries_before_completion() {
    let mut state = ChatStreamState::default();
    let completed: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.completed",
        "response": {
            "output": [{
                "type": "reasoning",
                "id": "rs_2",
                "summary": [{"type": "summary_text", "text": "Checked the inputs."}]
            }]
        }
    }))
    .expect("completion should deserialize");

    let outputs = state.observe(&completed).expect("fallback should decode");
    assert!(matches!(
        outputs.as_slice(),
        [
            StreamOutput::ReasoningSummaryStarted { key, .. },
            StreamOutput::ReasoningSummaryCompleted { key: completed_key, text },
            StreamOutput::Completed(_)
        ] if key == "reasoning:rs_2:0"
            && completed_key == "reasoning:rs_2:0"
            && text == "Checked the inputs."
    ));

    let duplicate = state
        .observe(&completed)
        .expect("duplicate completion should decode");
    assert!(matches!(duplicate.as_slice(), [StreamOutput::Completed(_)]));
}

#[test]
fn output_item_phases_are_preserved_for_text_deltas() {
    let mut state = ChatStreamState::default();
    let commentary: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_item.added",
        "item": {"type": "message", "id": "msg_1", "phase": "commentary"}
    }))
    .expect("commentary item should deserialize");
    state
        .observe(&commentary)
        .expect("commentary should decode");
    let delta: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_text.delta",
        "item_id": "msg_1",
        "delta": "Working"
    }))
    .expect("commentary delta should deserialize");
    assert!(matches!(
        state
            .observe(&delta)
            .expect("commentary delta should decode")
            .as_slice(),
        [StreamOutput::TextWithPhase {
            phase: AssistantTextPhase::Commentary,
            ..
        }]
    ));

    let final_item: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_item.done",
        "item": {"type": "message", "id": "msg_1", "phase": "final_answer"}
    }))
    .expect("final item should deserialize");
    state
        .observe(&final_item)
        .expect("final item should decode");
    let final_delta: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_text.delta",
        "item_id": "msg_1",
        "delta": "Done"
    }))
    .expect("final delta should deserialize");
    assert!(matches!(
        state
            .observe(&final_delta)
            .expect("final delta should decode")
            .as_slice(),
        [StreamOutput::TextWithPhase {
            phase: AssistantTextPhase::FinalAnswer,
            ..
        }]
    ));
}

#[test]
fn fallback_ignores_encrypted_or_non_summary_reasoning_content() {
    let output = vec![serde_json::json!({
        "type": "reasoning",
        "id": "rs_3",
        "encrypted_content": "opaque",
        "summary": [{"type": "reasoning_text", "text": "private"}]
    })];
    assert_eq!(wire::reasoning_summary_parts(&output).count(), 0);
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
