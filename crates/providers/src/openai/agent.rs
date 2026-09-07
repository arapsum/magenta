use std::collections::HashMap;

use futures_util::{
    StreamExt as _,
    io::{AsyncBufReadExt as _, BufReader},
};
use http_client::StatusCode;
use magenta_core::{
    AgentContinuation, AgentProvider, AgentProviderEvent, AgentProviderStream, AgentRequest,
    AgentResumeRequest, AgentToolCall, GenerationOutcome, ProviderError, ProviderErrorKind,
};

use super::{OpenAiProvider, StreamEvent, wire::ResponsePayload, wire::ResponsesRequest};

impl AgentProvider for OpenAiProvider {
    fn start(&self, request: AgentRequest) -> AgentProviderStream {
        let provider = self.clone();
        Box::pin(async_stream::try_stream! {
            let wire_request = ResponsesRequest::from_agent_request(&request).map_err(|message| {
                super::provider_error(
                    ProviderErrorKind::InvalidRequest,
                    super::OpenAiProviderError::Protocol(message),
                )
            })?;
            let stream = provider.agent_stream_inner(
                wire_request,
                request.generation.model.0,
                request.generation.effort,
            ).await?;
            futures_util::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                yield event?;
            }
        })
    }

    fn resume(&self, request: AgentResumeRequest) -> AgentProviderStream {
        let provider = self.clone();
        Box::pin(async_stream::try_stream! {
            let wire_request = ResponsesRequest::from_resume(
                &request,
                &request.continuation.model.0,
                &request.continuation.effort,
            ).map_err(|message| {
                super::provider_error(
                    ProviderErrorKind::InvalidRequest,
                    super::OpenAiProviderError::Protocol(message),
                )
            })?;
            let stream = provider.agent_stream_inner(
                wire_request,
                request.continuation.model.0,
                request.continuation.effort,
            ).await?;
            futures_util::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                yield event?;
            }
        })
    }
}

impl OpenAiProvider {
    async fn agent_stream_inner(
        &self,
        request: ResponsesRequest,
        model: String,
        effort: magenta_core::EffortLevel,
    ) -> Result<
        impl futures_util::Stream<Item = Result<AgentProviderEvent, ProviderError>>,
        ProviderError,
    > {
        let input = request.input.as_array().cloned().ok_or_else(|| {
            super::provider_error(
                ProviderErrorKind::InvalidRequest,
                super::OpenAiProviderError::Protocol(
                    "agent request input must be an array".to_owned(),
                ),
            )
        })?;
        let mut access_token = self.auth.access_token().await?;
        let mut response = self.send_responses(&access_token, &request).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            access_token = self.auth.force_refresh(&access_token).await?;
            response = self.send_responses(&access_token, &request).await?;
        }
        if !response.status().is_success() {
            return Err(self.http_error(response).await);
        }

        Ok(agent_response_stream(response, model, effort, input))
    }
}

fn agent_response_stream(
    response: http_client::Response<http_client::AsyncBody>,
    model: String,
    effort: magenta_core::EffortLevel,
    input: Vec<serde_json::Value>,
) -> impl futures_util::Stream<Item = Result<AgentProviderEvent, ProviderError>> {
    async_stream::try_stream! {
        let mut decoder = super::sse::EventDecoder::default();
        let mut reader = BufReader::new(response.into_body());
        let mut line = String::new();
        let mut state = AgentStreamState::with_input(input);
        let mut terminal = false;
        yield AgentProviderEvent::Started;

        loop {
            line.clear();
            let count = reader.read_line(&mut line).await.map_err(|error| {
                super::provider_error(
                    ProviderErrorKind::Transport,
                    super::OpenAiProviderError::Transport(error.to_string()),
                )
            })?;
            if count == 0 {
                break;
            }
            if let Some(event) = decoder.push_line(&line) {
                if terminal {
                    Err::<(), _>(super::provider_error(
                        ProviderErrorKind::Protocol,
                        super::OpenAiProviderError::Protocol(
                            "received data after the agent step completed".to_owned(),
                        ),
                    ))?;
                }
                if let Some(output) =
                    decode_agent_event(&event.data, &model, &effort, &mut state)?
                {
                    match output {
                        StreamOutput::Text(delta) => {
                            yield AgentProviderEvent::TextDelta(delta);
                        }
                        StreamOutput::AgentTools { calls, continuation } => {
                            terminal = true;
                            for call in calls {
                                yield AgentProviderEvent::ToolCall {
                                    call,
                                    continuation: continuation.clone(),
                                };
                            }
                            break;
                        }
                        StreamOutput::Completed(outcome) => {
                            terminal = true;
                            yield AgentProviderEvent::Completed(outcome);
                            break;
                        }
                    }
                }
            }
        }

        if !terminal {
            let output = decoder
                .finish()
                .map(|event| decode_agent_event(&event.data, &model, &effort, &mut state))
                .transpose()?
                .flatten();
            match output {
                Some(StreamOutput::Text(delta)) => {
                    yield AgentProviderEvent::TextDelta(delta);
                }
                Some(StreamOutput::AgentTools { calls, continuation }) => {
                    terminal = true;
                    for call in calls {
                        yield AgentProviderEvent::ToolCall {
                            call,
                            continuation: continuation.clone(),
                        };
                    }
                }
                Some(StreamOutput::Completed(outcome)) => {
                    terminal = true;
                    yield AgentProviderEvent::Completed(outcome);
                }
                None => {}
            }
        }

        if !terminal {
            Err::<(), _>(super::provider_error(
                ProviderErrorKind::Protocol,
                super::OpenAiProviderError::IncompleteStream,
            ))?;
        }
    }
}

fn decode_agent_event(
    data: &str,
    model: &str,
    effort: &magenta_core::EffortLevel,
    state: &mut AgentStreamState,
) -> Result<Option<StreamOutput>, ProviderError> {
    let event = serde_json::from_str::<StreamEvent>(data).map_err(|error| {
        super::provider_error(
            ProviderErrorKind::Protocol,
            super::OpenAiProviderError::Protocol(error.to_string()),
        )
    })?;
    state.observe(&event);
    match event.kind.as_str() {
        "response.output_text.delta" | "response.refusal.delta" => Ok(event
            .delta
            .filter(|delta| !delta.is_empty())
            .map(StreamOutput::Text)),
        "response.completed" | "response.done" | "response.incomplete" => {
            let response = event.response.as_ref().ok_or_else(|| {
                super::provider_error(
                    ProviderErrorKind::Protocol,
                    super::OpenAiProviderError::Protocol(
                        "agent completion did not contain a response payload".to_owned(),
                    ),
                )
            })?;
            let output = state.response_output(response);
            let calls = function_calls(&output)?;
            if calls.is_empty() {
                Ok(Some(StreamOutput::Completed(GenerationOutcome::new(
                    super::parse_finish_reason(response),
                    super::usage(response),
                ))))
            } else {
                let continuation = state.continuation_items(output);
                let payload = serde_json::to_vec(&continuation).map_err(|error| {
                    super::provider_error(
                        ProviderErrorKind::Protocol,
                        super::OpenAiProviderError::Protocol(error.to_string()),
                    )
                })?;
                Ok(Some(StreamOutput::AgentTools {
                    calls,
                    continuation: AgentContinuation {
                        provider: super::auth::openai_provider(),
                        model: magenta_core::ModelId::new(model),
                        effort: effort.clone(),
                        payload,
                    },
                }))
            }
        }
        "response.failed" | "error" | "response.error" => {
            let error = event
                .error
                .as_ref()
                .or_else(|| {
                    event
                        .response
                        .as_ref()
                        .and_then(|response| response.error.as_ref())
                })
                .map_or_else(
                    || "the provider reported an unspecified error".to_owned(),
                    super::response_error_detail,
                );
            Err(super::provider_error(
                ProviderErrorKind::Protocol,
                super::OpenAiProviderError::StreamFailed(error),
            ))
        }
        _ => Ok(None),
    }
}

fn function_calls(output: &[serde_json::Value]) -> Result<Vec<AgentToolCall>, ProviderError> {
    output
        .iter()
        .filter(|item| {
            item.get("type").and_then(serde_json::Value::as_str) == Some("function_call")
        })
        .map(|item| {
            let id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| protocol_error("function call had no call_id"))?;
            let name = item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| protocol_error("function call had no name"))?;
            let arguments = item
                .get("arguments")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| protocol_error("function call had no arguments"))?;
            Ok(AgentToolCall {
                id: id.to_owned(),
                name: name.to_owned(),
                arguments: arguments.to_owned(),
            })
        })
        .collect()
}

#[derive(Default)]
struct AgentStreamState {
    input_items: Vec<serde_json::Value>,
    output_items: Vec<CapturedItem>,
    arguments: HashMap<String, String>,
}

impl AgentStreamState {
    fn with_input(input_items: Vec<serde_json::Value>) -> Self {
        Self {
            input_items,
            ..Self::default()
        }
    }

    fn observe(&mut self, event: &StreamEvent) {
        match event.kind.as_str() {
            "response.output_item.added" | "response.output_item.done" => {
                if let Some(item) = event.item.clone() {
                    self.upsert_item(item, event.output_index);
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = event.delta.as_deref() {
                    self.arguments
                        .entry(event_key(event))
                        .or_default()
                        .push_str(delta);
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = event.arguments.as_deref() {
                    self.arguments
                        .insert(event_key(event), arguments.to_owned());
                }
            }
            _ => {}
        }

        for item in &mut self.output_items {
            if !is_function_call(&item.value) {
                continue;
            }
            if let Some(arguments) = self.arguments.get(&item.key) {
                item.value["arguments"] = serde_json::Value::String(arguments.clone());
            }
        }
    }

    fn response_output(&self, response: &ResponsePayload) -> Vec<serde_json::Value> {
        if response.output.is_empty() {
            return self
                .output_items
                .iter()
                .map(|item| item.value.clone())
                .collect();
        }

        if response.output.iter().any(is_function_call) {
            return response.output.clone();
        }

        let mut output = response.output.clone();
        output.extend(
            self.output_items
                .iter()
                .filter(|item| is_function_call(&item.value))
                .map(|item| item.value.clone()),
        );
        output
    }

    fn continuation_items(&self, output: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        let mut items = self.input_items.clone();
        items.extend(output);
        items
    }

    fn upsert_item(&mut self, item: serde_json::Value, output_index: Option<u64>) {
        let key = item_key(&item, output_index);
        let position = key.as_deref().and_then(|key| {
            self.output_items
                .iter()
                .position(|candidate| candidate.key == key)
        });
        if let Some(position) = position {
            self.output_items[position].value = item;
        } else {
            self.output_items.push(CapturedItem {
                key: key.unwrap_or_else(|| format!("item:{}", self.output_items.len())),
                value: item,
            });
        }
    }
}

fn event_key(event: &StreamEvent) -> String {
    event.item_id.as_deref().map_or_else(
        || {
            event
                .output_index
                .map_or_else(|| "current".to_owned(), |index| format!("index:{index}"))
        },
        |id| format!("id:{id}"),
    )
}

fn item_key(item: &serde_json::Value, output_index: Option<u64>) -> Option<String> {
    item.get("id")
        .and_then(serde_json::Value::as_str)
        .map(|id| format!("id:{id}"))
        .or_else(|| output_index.map(|index| format!("index:{index}")))
}

fn is_function_call(item: &serde_json::Value) -> bool {
    item.get("type").and_then(serde_json::Value::as_str) == Some("function_call")
}

struct CapturedItem {
    key: String,
    value: serde_json::Value,
}

fn protocol_error(message: &str) -> ProviderError {
    super::provider_error(
        ProviderErrorKind::Protocol,
        super::OpenAiProviderError::Protocol(message.to_owned()),
    )
}

enum StreamOutput {
    Text(String),
    AgentTools {
        calls: Vec<AgentToolCall>,
        continuation: AgentContinuation,
    },
    Completed(GenerationOutcome),
}

#[cfg(test)]
mod tests {
    use magenta_core::{AgentToolCall, EffortLevel};

    use super::{AgentStreamState, StreamOutput, decode_agent_event};

    const MODEL: &str = "gpt-5.6-luna";

    #[test]
    fn streamed_function_call_arguments_are_reassembled() {
        let effort = EffortLevel::XHigh;
        let mut state = AgentStreamState::default();

        let added = serde_json::json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "create_file",
                "arguments": ""
            }
        });
        assert!(
            decode_agent_event(&added.to_string(), MODEL, &effort, &mut state)
                .expect("output item should decode")
                .is_none()
        );

        for delta in [
            r#"{"path":"hello-gtk/"#,
            r#"main.c","content":"Hello, World"}"#,
        ] {
            let event = serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "item_1",
                "delta": delta
            });
            assert!(
                decode_agent_event(&event.to_string(), MODEL, &effort, &mut state)
                    .expect("argument delta should decode")
                    .is_none()
            );
        }

        let completed = serde_json::json!({
            "type": "response.completed",
            "response": { "output": [] }
        });
        let output = decode_agent_event(&completed.to_string(), MODEL, &effort, &mut state)
            .expect("completion should decode")
            .expect("completion should contain a tool call");

        let StreamOutput::AgentTools {
            calls,
            continuation,
        } = output
        else {
            panic!("expected a tool call output");
        };
        assert_eq!(
            calls,
            vec![AgentToolCall {
                id: "call_1".to_owned(),
                name: "create_file".to_owned(),
                arguments: r#"{"path":"hello-gtk/main.c","content":"Hello, World"}"#.to_owned(),
            }]
        );

        let payload: Vec<serde_json::Value> =
            serde_json::from_slice(&continuation.payload).expect("continuation should be JSON");
        assert_eq!(payload[0]["type"], "function_call");
        assert_eq!(payload[0]["call_id"], "call_1");
        assert_eq!(payload[0]["arguments"], calls[0].arguments);
    }

    #[test]
    fn completed_response_function_call_is_forwarded() {
        let effort = EffortLevel::High;
        let user_input = serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Inspect the project"}]
        });
        let mut state = AgentStreamState::with_input(vec![user_input.clone()]);
        let completed = serde_json::json!({
            "type": "response.completed",
            "response": {
                "output": [{
                    "type": "function_call",
                    "call_id": "call_2",
                    "name": "list_files",
                    "arguments": "{\"path\":\".\"}"
                }]
            }
        });

        let output = decode_agent_event(&completed.to_string(), MODEL, &effort, &mut state)
            .expect("completion should decode")
            .expect("completion should contain a tool call");
        let StreamOutput::AgentTools {
            calls,
            continuation,
        } = output
        else {
            panic!("expected a tool call output");
        };
        assert_eq!(calls[0].name, "list_files");
        assert_eq!(calls[0].arguments, r#"{"path":"."}"#);

        let payload: Vec<serde_json::Value> =
            serde_json::from_slice(&continuation.payload).expect("continuation should be JSON");
        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0], user_input);
        assert_eq!(payload[1]["type"], "function_call");
    }
}
