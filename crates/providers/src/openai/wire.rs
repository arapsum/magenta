use std::fs;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use magenta_core::{
    AgentRequest, AgentResumeRequest, AgentToolDefinition, Attachment, EffortLevel, FinishReason,
    ModelDescriptor, ModelId, ProviderId, TokenUsage,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: serde_json::Value,
    pub stream: bool,
    pub store: bool,
    pub reasoning: Reasoning,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<FunctionTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum InputItem {
    #[serde(rename = "message")]
    Message {
        role: String,
        content: Vec<InputContent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
}

#[derive(Debug, Serialize, Clone)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub strict: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum InputContent {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "output_text")]
    OutputText {
        text: String,
        annotations: Vec<serde_json::Value>,
    },
    #[serde(rename = "input_image")]
    InputImage {
        image_url: String,
        detail: &'static str,
    },
}

const MAX_IMAGES_PER_MESSAGE: usize = 4;
const MAX_IMAGE_REQUEST_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct Reasoning {
    pub effort: String,
    pub summary: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub output_index: Option<u64>,
    #[serde(default)]
    pub item: Option<serde_json::Value>,
    #[serde(default)]
    pub response: Option<ResponsePayload>,
    #[serde(default)]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Deserialize)]
pub struct ResponsePayload {
    #[serde(default)]
    pub usage: Option<ResponseUsage>,
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(default)]
    pub error: Option<ResponseError>,
    #[serde(default)]
    pub output: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub struct IncompleteDetails {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ModelsResponse {
    Models { models: Vec<ModelInfo> },
    Data { data: Vec<ModelInfo> },
    List(Vec<ModelInfo>),
}

impl ModelsResponse {
    fn into_models(self) -> Vec<ModelInfo> {
        match self {
            Self::Models { models } | Self::Data { data: models } | Self::List(models) => models,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ModelInfo {
    #[serde(alias = "id")]
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_reasoning_level: Option<String>,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Deserialize)]
pub struct ReasoningLevel {
    pub effort: String,
}

impl ResponsesRequest {
    pub fn from_request(
        model: &str,
        effort: &EffortLevel,
        messages: &[magenta_core::Message],
    ) -> Result<Self, String> {
        let mut input = Vec::with_capacity(messages.len());
        let mut has_user_message = false;

        let mut total_image_bytes = 0_u64;

        for message in messages {
            if message.content.trim().is_empty() && message.attachments.is_empty() {
                continue;
            }

            let item = match message.role {
                magenta_core::MessageRole::User => {
                    has_user_message = true;
                    let mut content = Vec::with_capacity(1 + message.attachments.len());
                    if !message.content.trim().is_empty() {
                        content.push(InputContent::InputText {
                            text: message.content.clone(),
                        });
                    }
                    content.extend(attachment_content(
                        &message.attachments,
                        &mut total_image_bytes,
                    )?);
                    InputItem::Message {
                        role: "user".to_owned(),
                        content,
                        status: None,
                    }
                }
                magenta_core::MessageRole::Assistant => InputItem::Message {
                    role: "assistant".to_owned(),
                    content: vec![InputContent::OutputText {
                        text: message.content.clone(),
                        annotations: Vec::new(),
                    }],
                    status: Some("completed".to_owned()),
                },
            };
            input.push(item);
        }

        if !has_user_message {
            return Err("the generation request did not contain a user message".into());
        }

        Ok(Self {
            model: model.to_owned(),
            input: serde_json::to_value(input).map_err(|error| error.to_string())?,
            stream: true,
            store: false,
            reasoning: Reasoning {
                effort: effort.wire_value().to_owned(),
                summary: "auto",
            },
            instructions: None,
            tools: Vec::new(),
            tool_choice: None,
        })
    }

    pub fn from_agent_request(request: &AgentRequest) -> Result<Self, String> {
        let mut wire = Self::from_request(
            &request.generation.model.0,
            &request.generation.effort,
            &request.messages,
        )?;
        wire.instructions = Some(request.instructions.clone());
        wire.tools = request
            .tools
            .iter()
            .map(FunctionTool::from_definition)
            .collect();
        wire.tool_choice = (!wire.tools.is_empty()).then_some("required");
        Ok(wire)
    }

    pub fn from_resume(
        request: &AgentResumeRequest,
        model: &str,
        effort: &EffortLevel,
    ) -> Result<Self, String> {
        let mut input =
            serde_json::from_slice::<Vec<serde_json::Value>>(&request.continuation.payload)
                .map_err(|error| format!("invalid provider continuation: {error}"))?;
        input.extend(request.outputs.iter().map(|output| {
            serde_json::json!({
                "type": "function_call_output",
                "call_id": output.call_id,
                "output": output.output,
            })
        }));
        Ok(Self {
            model: model.to_owned(),
            input: serde_json::Value::Array(input),
            stream: true,
            store: false,
            reasoning: Reasoning {
                effort: effort.wire_value().to_owned(),
                summary: "auto",
            },
            instructions: Some(request.instructions.clone()),
            tools: request
                .tools
                .iter()
                .map(FunctionTool::from_definition)
                .collect(),
            // The first request must enter the workspace loop. Once a tool result
            // is available, the model needs to be able to finish with text.
            tool_choice: None,
        })
    }
}

impl FunctionTool {
    fn from_definition(definition: &AgentToolDefinition) -> Self {
        Self {
            kind: "function",
            name: definition.name.clone(),
            description: definition.description.clone(),
            parameters: definition.parameters.clone(),
            strict: true,
        }
    }
}

fn attachment_content(
    attachments: &[Attachment],
    total_image_bytes: &mut u64,
) -> Result<Vec<InputContent>, String> {
    if attachments.len() > MAX_IMAGES_PER_MESSAGE {
        return Err("a message can contain at most four images".to_owned());
    }

    attachments
        .iter()
        .map(|attachment| {
            if !matches!(
                attachment.mime_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp" | "image/gif"
            ) {
                return Err("an attached image has an unsupported file type".to_owned());
            }

            *total_image_bytes = total_image_bytes
                .checked_add(attachment.byte_size)
                .ok_or_else(|| "attached images exceed Magenta's request limit".to_owned())?;
            if *total_image_bytes > MAX_IMAGE_REQUEST_BYTES {
                return Err("attached images exceed Magenta's 32 MiB request limit".to_owned());
            }

            let bytes = fs::read(&attachment.path)
                .map_err(|_| "an attached image is no longer available locally".to_owned())?;
            let actual_size = u64::try_from(bytes.len())
                .map_err(|_| "an attached image is too large to send".to_owned())?;
            if actual_size != attachment.byte_size {
                return Err("an attached image changed after it was saved".to_owned());
            }

            Ok(InputContent::InputImage {
                image_url: format!(
                    "data:{};base64,{}",
                    attachment.mime_type,
                    STANDARD.encode(bytes)
                ),
                detail: "auto",
            })
        })
        .collect()
}

pub fn model_descriptors(response: ModelsResponse) -> Vec<ModelDescriptor> {
    let mut models = response
        .into_models()
        .into_iter()
        .filter(|model| {
            model.visibility.as_deref().is_none_or(|value| {
                !value.eq_ignore_ascii_case("hide") && !value.eq_ignore_ascii_case("hidden")
            })
        })
        .filter_map(model_descriptor)
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.id.0.cmp(&right.id.0))
    });
    models
}

fn model_descriptor(model: ModelInfo) -> Option<ModelDescriptor> {
    let display_name = model.display_name.unwrap_or_else(|| model.slug.clone());
    if model.slug.trim().is_empty() || display_name.trim().is_empty() {
        return None;
    }

    let mut supported_efforts = model
        .supported_reasoning_levels
        .into_iter()
        .filter_map(|level| EffortLevel::from_wire(&level.effort))
        .collect::<Vec<_>>();
    supported_efforts.sort_by_key(effort_order);
    supported_efforts.dedup();
    if supported_efforts.is_empty() {
        supported_efforts.extend(EffortLevel::ALL);
    }

    let default_effort = model
        .default_reasoning_level
        .as_deref()
        .and_then(EffortLevel::from_wire)
        .filter(|effort| supported_efforts.contains(effort))
        .unwrap_or_else(|| {
            supported_efforts
                .first()
                .cloned()
                .unwrap_or(EffortLevel::Medium)
        });

    Some(ModelDescriptor {
        provider: ProviderId::new("openai"),
        id: ModelId::new(model.slug),
        display_name,
        description: model.description,
        priority: model.priority,
        default_effort,
        supported_efforts,
    })
}

pub fn parse_finish_reason(response: &ResponsePayload) -> FinishReason {
    match response
        .incomplete_details
        .as_ref()
        .and_then(|details| details.reason.as_deref())
    {
        Some("max_output_tokens") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(reason) => FinishReason::Other(reason.to_owned()),
        None => FinishReason::Stop,
    }
}

pub fn usage(response: &ResponsePayload) -> Option<TokenUsage> {
    response.usage.as_ref().map(|usage| TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    })
}

const fn effort_order(effort: &EffortLevel) -> u8 {
    match effort {
        EffortLevel::None => 0,
        EffortLevel::Minimal => 1,
        EffortLevel::Low => 2,
        EffortLevel::Medium => 3,
        EffortLevel::High => 4,
        EffortLevel::XHigh => 5,
        EffortLevel::Max => 6,
        EffortLevel::Custom { .. } => 7,
    }
}

#[cfg(test)]
mod tests {
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
}
