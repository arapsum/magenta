use std::fs;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use magenta_core::{
    AgentRequest, AgentResumeRequest, AgentToolDefinition, Attachment, EffortLevel, FinishReason,
    GenerationLimits, ModelDescriptor, ModelId, ProviderId, TokenUsage,
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
    #[serde(default, alias = "context_window")]
    pub context_window_tokens: Option<u64>,
    #[serde(default, alias = "max_output")]
    pub max_output_tokens: Option<u64>,
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

    let limits = valid_limits(model.context_window_tokens, model.max_output_tokens)
        .unwrap_or_else(|| known_model_limits(&model.slug));
    Some(ModelDescriptor {
        provider: ProviderId::new("openai"),
        id: ModelId::new(model.slug),
        display_name,
        description: model.description,
        priority: model.priority,
        default_effort,
        supported_efforts,
        limits,
    })
}

fn valid_limits(context: Option<u64>, output: Option<u64>) -> Option<GenerationLimits> {
    let (context_window_tokens, max_output_tokens) = (context?, output?);
    (context_window_tokens > 0
        && max_output_tokens > 0
        && max_output_tokens < context_window_tokens)
        .then_some(GenerationLimits {
            context_window_tokens,
            max_output_tokens,
        })
}

fn known_model_limits(model: &str) -> GenerationLimits {
    let model = model.to_ascii_lowercase();
    if model.starts_with("gpt-5.4-mini") {
        GenerationLimits {
            context_window_tokens: 400_000,
            max_output_tokens: 128_000,
        }
    } else if model.starts_with("gpt-6-astra")
        || model.starts_with("gpt-5.6-")
        || model.starts_with("gpt-5.5")
        || model.starts_with("gpt-5.4")
    {
        GenerationLimits {
            context_window_tokens: 1_050_000,
            max_output_tokens: 128_000,
        }
    } else {
        GenerationLimits::default()
    }
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
#[path = "wire/tests.rs"]
mod tests;
