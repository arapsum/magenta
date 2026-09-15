use std::collections::HashMap;

use futures_util::{
    StreamExt as _,
    io::{AsyncBufReadExt as _, BufReader},
};
use http_client::StatusCode;
use magenta_core::{
    AgentContinuation, AgentProvider, AgentProviderEvent, AgentProviderStream, AgentRequest,
    AgentResumeRequest, AgentToolCall, AssistantTextPhase, GenerationOutcome, ProviderError,
    ProviderErrorKind,
};

use super::{
    OpenAiProvider, StreamEvent, assistant_text_phase,
    wire::{ResponsePayload, ResponsesRequest, reasoning_summary_parts},
};

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
                    for event in agent_provider_events(&mut state, Some(output), &mut terminal) {
                        yield event;
                    }
                    if terminal {
                        break;
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
            for event in agent_provider_events(&mut state, output, &mut terminal) {
                yield event;
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

fn agent_provider_events(
    state: &mut AgentStreamState,
    output: Option<StreamOutput>,
    terminal: &mut bool,
) -> Vec<AgentProviderEvent> {
    let mut events = Vec::new();
    for pending in state.take_pending() {
        match pending {
            StreamOutput::ReasoningSummaryStarted { key, title } => {
                events.push(AgentProviderEvent::ReasoningSummaryStarted { key, title });
            }
            StreamOutput::ReasoningSummaryCompleted { key, text } => {
                events.push(AgentProviderEvent::ReasoningSummaryCompleted { key, text });
            }
            _ => {}
        }
    }

    if let Some(output) = output {
        match output {
            StreamOutput::Text(delta) => events.push(AgentProviderEvent::TextDelta(delta)),
            StreamOutput::TextWithPhase { delta, phase } => {
                events.push(AgentProviderEvent::TextDeltaWithPhase { delta, phase });
            }
            StreamOutput::ReasoningSummaryStarted { key, title } => {
                events.push(AgentProviderEvent::ReasoningSummaryStarted { key, title });
            }
            StreamOutput::ReasoningSummaryDelta { key, delta } => {
                events.push(AgentProviderEvent::ReasoningSummaryDelta { key, delta });
            }
            StreamOutput::ReasoningSummaryCompleted { key, text } => {
                events.push(AgentProviderEvent::ReasoningSummaryCompleted { key, text });
            }
            StreamOutput::AgentTools {
                calls,
                continuation,
            } => {
                *terminal = true;
                events.extend(calls.into_iter().map(|call| AgentProviderEvent::ToolCall {
                    call,
                    continuation: continuation.clone(),
                }));
            }
            StreamOutput::Completed(outcome) => {
                *terminal = true;
                events.push(AgentProviderEvent::Completed(outcome));
            }
        }
    }
    events
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
        "response.output_text.delta" | "response.refusal.delta" => {
            Ok(decode_agent_text(&event, state))
        }
        "response.reasoning_summary_part.added" => Ok(decode_summary_part(&event, state)),
        "response.reasoning_summary_text.delta" => Ok(decode_summary_delta(&event, state)),
        "response.reasoning_summary_text.done" => Ok(decode_summary_done(&event, state)),
        "response.completed" | "response.done" | "response.incomplete" => {
            decode_agent_completion(&event, model, effort, state)
        }
        "response.failed" | "error" | "response.error" => Err(agent_stream_error(&event)),
        _ => Ok(None),
    }
}

fn decode_agent_text(event: &StreamEvent, state: &AgentStreamState) -> Option<StreamOutput> {
    event
        .delta
        .as_ref()
        .filter(|delta| !delta.is_empty())
        .map(|delta| {
            let phase = event
                .phase
                .as_deref()
                .and_then(assistant_text_phase)
                .or_else(|| state.phase_for(event.item_id.as_deref()));
            phase.map_or_else(
                || StreamOutput::Text(delta.clone()),
                |phase| StreamOutput::TextWithPhase {
                    delta: delta.clone(),
                    phase,
                },
            )
        })
}

fn decode_summary_part(event: &StreamEvent, state: &mut AgentStreamState) -> Option<StreamOutput> {
    AgentStreamState::summary_key(event)
        .filter(|key| state.begin_summary(key))
        .map(|key| StreamOutput::ReasoningSummaryStarted {
            key,
            title: "Thinking".to_owned(),
        })
}

fn decode_summary_delta(event: &StreamEvent, state: &mut AgentStreamState) -> Option<StreamOutput> {
    let key = AgentStreamState::summary_key(event)?;
    let delta = event.delta.as_deref().filter(|delta| !delta.is_empty())?;
    if !state
        .summaries
        .get(&key)
        .is_some_and(|summary| summary.started)
    {
        state.pending.push(StreamOutput::ReasoningSummaryStarted {
            key: key.clone(),
            title: "Thinking".to_owned(),
        });
    }
    state.append_summary(&key, delta);
    Some(StreamOutput::ReasoningSummaryDelta {
        key,
        delta: delta.to_owned(),
    })
}

fn decode_summary_done(event: &StreamEvent, state: &mut AgentStreamState) -> Option<StreamOutput> {
    let key = AgentStreamState::summary_key(event)?;
    let started = state
        .summaries
        .get(&key)
        .is_some_and(|summary| summary.started);
    let (key, text) = state.finish_summary(&key, event.text.as_deref())?;
    if !started {
        state.pending.push(StreamOutput::ReasoningSummaryStarted {
            key: key.clone(),
            title: "Thinking".to_owned(),
        });
    }
    Some(StreamOutput::ReasoningSummaryCompleted { key, text })
}

fn decode_agent_completion(
    event: &StreamEvent,
    model: &str,
    effort: &magenta_core::EffortLevel,
    state: &mut AgentStreamState,
) -> Result<Option<StreamOutput>, ProviderError> {
    let response = event.response.as_ref().ok_or_else(|| {
        super::provider_error(
            ProviderErrorKind::Protocol,
            super::OpenAiProviderError::Protocol(
                "agent completion did not contain a response payload".to_owned(),
            ),
        )
    })?;
    let output = state.response_output(response);
    state.queue_reasoning_fallback(&output);
    let calls = function_calls(&output)?;
    if calls.is_empty() {
        return Ok(Some(StreamOutput::Completed(GenerationOutcome::new(
            super::parse_finish_reason(response),
            super::usage(response),
        ))));
    }

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

fn agent_stream_error(event: &StreamEvent) -> ProviderError {
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
    super::provider_error(
        ProviderErrorKind::Protocol,
        super::OpenAiProviderError::StreamFailed(error),
    )
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
    phases: HashMap<String, AssistantTextPhase>,
    summaries: HashMap<String, AgentSummaryState>,
    pending: Vec<StreamOutput>,
}

#[derive(Default)]
struct AgentSummaryState {
    text: String,
    started: bool,
    completed: bool,
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
                    if item.get("type").and_then(serde_json::Value::as_str) == Some("message")
                        && let Some(id) = item.get("id").and_then(serde_json::Value::as_str)
                        && let Some(phase) = item
                            .get("phase")
                            .and_then(serde_json::Value::as_str)
                            .and_then(assistant_text_phase)
                    {
                        self.phases.insert(id.to_owned(), phase);
                    }
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

    fn phase_for(&self, item_id: Option<&str>) -> Option<AssistantTextPhase> {
        item_id.and_then(|item_id| self.phases.get(item_id).copied())
    }

    fn summary_key(event: &StreamEvent) -> Option<String> {
        Some(format!(
            "reasoning:{}:{}",
            event.item_id.as_deref()?,
            event.summary_index?
        ))
    }

    fn begin_summary(&mut self, key: &str) -> bool {
        let state = self.summaries.entry(key.to_owned()).or_default();
        if state.started {
            return false;
        }
        state.started = true;
        true
    }

    fn append_summary(&mut self, key: &str, delta: &str) {
        let state = self.summaries.entry(key.to_owned()).or_default();
        state.started = true;
        state.text.push_str(delta);
    }

    fn finish_summary(&mut self, key: &str, text: Option<&str>) -> Option<(String, String)> {
        let state = self.summaries.entry(key.to_owned()).or_default();
        if state.completed {
            return None;
        }
        state.started = true;
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            text.clone_into(&mut state.text);
        }
        state.completed = true;
        Some((key.to_owned(), state.text.clone()))
    }

    fn queue_reasoning_fallback(&mut self, output: &[serde_json::Value]) {
        for (item_id, summary_index, text) in reasoning_summary_parts(output) {
            let key = format!("reasoning:{item_id}:{summary_index}");
            let state = self.summaries.entry(key.clone()).or_default();
            if state.completed {
                continue;
            }
            if !state.started {
                state.started = true;
                self.pending.push(StreamOutput::ReasoningSummaryStarted {
                    key: key.clone(),
                    title: "Thinking".to_owned(),
                });
            }
            state.text.clone_from(&text);
            state.completed = true;
            self.pending
                .push(StreamOutput::ReasoningSummaryCompleted { key, text });
        }
    }

    fn take_pending(&mut self) -> Vec<StreamOutput> {
        std::mem::take(&mut self.pending)
    }

    fn response_output(&self, response: &ResponsePayload) -> Vec<serde_json::Value> {
        if response.output.is_empty() {
            return self
                .output_items
                .iter()
                .map(|item| item.value.clone())
                .collect();
        }

        let mut output = response.output.clone();
        for item in &self.output_items {
            let duplicate = output.iter().any(|candidate| {
                item_key(candidate, None).is_some()
                    && item_key(candidate, None) == Some(item.key.clone())
            });
            if !duplicate {
                output.push(item.value.clone());
            }
        }
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
    TextWithPhase {
        delta: String,
        phase: AssistantTextPhase,
    },
    ReasoningSummaryStarted {
        key: String,
        title: String,
    },
    ReasoningSummaryDelta {
        key: String,
        delta: String,
    },
    ReasoningSummaryCompleted {
        key: String,
        text: String,
    },
    AgentTools {
        calls: Vec<AgentToolCall>,
        continuation: AgentContinuation,
    },
    Completed(GenerationOutcome),
}

#[cfg(test)]
#[path = "../../test/openai/agent.rs"]
mod tests;
