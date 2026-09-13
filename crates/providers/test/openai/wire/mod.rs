use std::fs;

use super::*;
use magenta_core::{
    AgentContinuation, AgentToolOutput, Attachment, ConversationId, GenerationConfig, Message,
    MessageId, MessageRole, MessageStatus,
};

fn message(role: MessageRole, content: &str) -> Message {
    Message {
        id: MessageId::new(1),
        conversation_id: ConversationId::new(1),
        role,
        content: content.to_owned(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        agent_activities: Vec::new(),
    }
}

#[test]
fn responses_request_encodes_user_and_assistant_history() {
    let request = ResponsesRequest::from_request(
        "gpt-5.4",
        &EffortLevel::High,
        &[
            message(MessageRole::User, "Hello"),
            message(MessageRole::Assistant, "Hi there"),
        ],
    )
    .expect("request should be valid");
    let value = serde_json::to_value(request).expect("request should serialize");

    assert_eq!(value["model"], "gpt-5.4");
    assert_eq!(value["stream"], true);
    assert_eq!(value["store"], false);
    assert_eq!(value["reasoning"]["effort"], "high");
    assert_eq!(value["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(value["input"][1]["content"][0]["type"], "output_text");
}

#[test]
fn agent_request_encodes_strict_workspace_tools_and_instructions() {
    let request = AgentRequest {
        generation: GenerationConfig::new(
            ProviderId::new("openai-codex"),
            ModelId::new("gpt-5.6-luna"),
            EffortLevel::High,
        ),
        messages: vec![message(MessageRole::User, "Inspect the workspace")],
        instructions: "Use only workspace tools.".to_owned(),
        tools: vec![AgentToolDefinition {
            name: "read_file".to_owned(),
            description: "Read one file.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
                "additionalProperties": false
            }),
            mutating: false,
            protected_read: true,
        }],
    };

    let value = serde_json::to_value(
        ResponsesRequest::from_agent_request(&request).expect("agent request should be valid"),
    )
    .expect("agent request should serialize");

    assert_eq!(value["instructions"], "Use only workspace tools.");
    assert_eq!(value["tools"][0]["type"], "function");
    assert_eq!(value["tools"][0]["name"], "read_file");
    assert_eq!(value["tools"][0]["strict"], true);
    assert_eq!(value["tools"][0]["parameters"]["required"][0], "path");
    assert_eq!(value["tool_choice"], "required");
}

#[test]
fn agent_resume_appends_tool_outputs_to_the_opaque_continuation() {
    let continuation = AgentContinuation {
        provider: ProviderId::new("openai-codex"),
        model: ModelId::new("gpt-5.6-luna"),
        effort: EffortLevel::Medium,
        payload: serde_json::to_vec(&vec![
            serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Inspect the workspace"}]
            }),
            serde_json::json!({
                "type": "function_call",
                "call_id": "call-1"
            }),
        ])
        .expect("continuation should serialize"),
    };
    let request = AgentResumeRequest {
        continuation,
        outputs: vec![AgentToolOutput {
            call_id: "call-1".to_owned(),
            output: "src/main.rs".to_owned(),
            is_error: false,
        }],
        instructions: "Use only workspace tools.".to_owned(),
        tools: Vec::new(),
    };

    let value = serde_json::to_value(
        ResponsesRequest::from_resume(&request, "gpt-5.6-luna", &EffortLevel::Medium)
            .expect("resume request should be valid"),
    )
    .expect("resume request should serialize");

    assert_eq!(value["input"][0]["type"], "message");
    assert_eq!(value["input"][0]["role"], "user");
    assert_eq!(value["input"][1]["type"], "function_call");
    assert_eq!(value["input"][2]["type"], "function_call_output");
    assert_eq!(value["input"][2]["call_id"], "call-1");
    assert_eq!(value["input"][2]["output"], "src/main.rs");
    assert!(value.get("tool_choice").is_none());
}

#[test]
fn responses_request_encodes_saved_images_as_data_urls() {
    let directory = tempfile::tempdir().expect("temporary directory should exist");
    let path = directory.path().join("reference.png");
    fs::write(&path, [0x89, b'P', b'N', b'G']).expect("test image should be written");
    let mut user = message(MessageRole::User, "What is shown here?");
    user.attachments.push(Attachment {
        name: "reference.png".to_owned(),
        path,
        mime_type: "image/png".to_owned(),
        byte_size: 4,
        managed: true,
    });

    let request = ResponsesRequest::from_request("gpt-5.6-luna", &EffortLevel::Medium, &[user])
        .expect("image request should be valid");
    let value = serde_json::to_value(request).expect("request should serialize");

    assert_eq!(value["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(value["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(value["input"][0]["content"][1]["detail"], "auto");
    assert_eq!(
        value["input"][0]["content"][1]["image_url"],
        "data:image/png;base64,iVBORw=="
    );
}

#[test]
fn responses_request_allows_an_image_without_prompt_text() {
    let directory = tempfile::tempdir().expect("temporary directory should exist");
    let path = directory.path().join("reference.jpeg");
    fs::write(&path, [0xFF, 0xD8, 0xFF]).expect("test image should be written");
    let mut user = message(MessageRole::User, "");
    user.attachments.push(Attachment {
        name: "reference.jpeg".to_owned(),
        path,
        mime_type: "image/jpeg".to_owned(),
        byte_size: 3,
        managed: true,
    });

    let request = ResponsesRequest::from_request("gpt-5.6-luna", &EffortLevel::Medium, &[user])
        .expect("image-only request should be valid");
    let value = serde_json::to_value(request).expect("request should serialize");

    assert_eq!(
        value["input"][0]["content"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(value["input"][0]["content"][0]["type"], "input_image");
}

#[test]
fn responses_request_rejects_images_over_the_request_limit() {
    let mut user = message(MessageRole::User, "Analyze this image");
    user.attachments.push(Attachment {
        name: "large.png".to_owned(),
        path: std::path::PathBuf::from("not-read.png"),
        mime_type: "image/png".to_owned(),
        byte_size: MAX_IMAGE_REQUEST_BYTES + 1,
        managed: true,
    });

    let error = ResponsesRequest::from_request("gpt-5.6-luna", &EffortLevel::Medium, &[user])
        .expect_err("oversized image requests must be rejected before reading files");

    assert_eq!(
        error,
        "attached images exceed Magenta's 32 MiB request limit"
    );
}

#[test]
fn response_events_map_deltas_completion_usage_and_incomplete_reasons() {
    let delta: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_text.delta",
        "delta": "hello"
    }))
    .expect("delta should deserialize");
    assert_eq!(delta.delta.as_deref(), Some("hello"));

    let completed: StreamEvent = serde_json::from_value(serde_json::json!({
        "type": "response.completed",
        "response": {"usage": {"input_tokens": 4, "output_tokens": 7}}
    }))
    .expect("completion should deserialize");
    let response = completed.response.expect("response payload should exist");
    assert_eq!(usage(&response).map(|usage| usage.output_tokens), Some(7));
    assert_eq!(parse_finish_reason(&response), FinishReason::Stop);

    let incomplete: ResponsePayload = serde_json::from_value(serde_json::json!({
        "incomplete_details": {"reason": "max_output_tokens"}
    }))
    .expect("incomplete response should deserialize");
    assert_eq!(parse_finish_reason(&incomplete), FinishReason::Length);
}

#[test]
fn model_catalog_accepts_the_codex_models_envelope() {
    let response: ModelsResponse = serde_json::from_value(serde_json::json!({
        "models": [{
            "slug": "gpt-5.4",
            "display_name": "GPT-5.4",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {"effort": "low"},
                {"effort": "medium"},
                {"effort": "high"},
                {"effort": "xhigh"},
                {"effort": "max"}
            ],
            "visibility": "list",
            "priority": 2
        }]
    }))
    .expect("the Codex models envelope should deserialize");
    let models = model_descriptors(response);

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.0, "gpt-5.4");
    assert_eq!(models[0].default_effort, EffortLevel::Medium);
    assert_eq!(models[0].supported_efforts, EffortLevel::ALL.to_vec());
    assert_eq!(models[0].limits.context_window_tokens, 1_050_000);
    assert_eq!(models[0].limits.max_output_tokens, 128_000);
}

#[test]
fn model_catalog_prefers_valid_provider_limits() {
    let response: ModelsResponse = serde_json::from_value(serde_json::json!({
        "models": [{
            "slug": "future-model",
            "context_window_tokens": 64000,
            "max_output_tokens": 8000
        }]
    }))
    .unwrap();
    let model = model_descriptors(response).pop().unwrap();
    assert_eq!(model.limits.context_window_tokens, 64_000);
    assert_eq!(model.limits.max_output_tokens, 8_000);
}

#[test]
fn model_catalog_keeps_visible_entries_with_non_list_visibility() {
    let response: ModelsResponse = serde_json::from_value(serde_json::json!({
        "data": [
            {"id": "gpt-visible", "visibility": "public"},
            {"id": "gpt-hidden", "visibility": "hidden"},
            {"id": "gpt-hidden-alias", "visibility": "HIDE"}
        ]
    }))
    .expect("the data models envelope should deserialize");

    let models = model_descriptors(response);

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.0, "gpt-visible");
}
