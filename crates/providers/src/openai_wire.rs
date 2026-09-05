use magenta_core::{EffortLevel, FinishReason, ModelDescriptor, ModelId, ProviderId, TokenUsage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: Vec<InputItem>,
    pub stream: bool,
    pub store: bool,
    pub reasoning: Reasoning,
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
}

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

        for message in messages {
            if !message.attachments.is_empty() {
                return Err(
                    "image attachments are not supported by the OpenAI provider yet".into(),
                );
            }
            if message.content.trim().is_empty() {
                continue;
            }

            let item = match message.role {
                magenta_core::MessageRole::User => {
                    has_user_message = true;
                    InputItem::Message {
                        role: "user".to_owned(),
                        content: vec![InputContent::InputText {
                            text: message.content.clone(),
                        }],
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
            input,
            stream: true,
            store: false,
            reasoning: Reasoning {
                effort: effort.wire_value().to_owned(),
                summary: "auto",
            },
        })
    }
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
    use super::*;
    use magenta_core::{ConversationId, Message, MessageId, MessageRole, MessageStatus};

    fn message(role: MessageRole, content: &str) -> Message {
        Message {
            id: MessageId::new(1),
            conversation_id: ConversationId::new(1),
            role,
            content: content.to_owned(),
            status: MessageStatus::Complete,
            attachments: Vec::new(),
            generation_outcome: None,
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
