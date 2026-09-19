use std::sync::{Arc, Mutex};

use async_channel::Receiver;
use futures_util::StreamExt as _;
use magenta_core::{
    AgentContinuation, AgentProviderEvent, AgentRequest, AgentResumeRequest, AgentRunEvent,
    AgentRunStream, AgentToolCall,
};

use super::{
    AgentStreamContext, ApprovalResponse, MAX_AGENT_ROUNDS, MAX_AGENT_TOOL_CALLS, agent_error, tool,
};
use tool::AgentRunPermissions;

pub fn agent_stream(
    context: AgentStreamContext,
    request: AgentRequest,
    approvals: Receiver<ApprovalResponse>,
) -> AgentRunStream {
    Box::pin(async_stream::try_stream! {
        let mut loop_guard = AgentLoopGuard::default();
        let provider_id = request.generation.provider.clone();

        let mut provider_stream = context.provider.start(request.clone());

        let mut first_started = false;

        let permissions = Arc::new(Mutex::new(AgentRunPermissions::default()));

        loop {
            let mut calls = Vec::new();
            let mut continuation = None;

            while let Some(event) = provider_stream.next().await {
                match observe_provider_event(&context, event?, &mut first_started).await {
                    ProviderEventAction::Ignore => {}
                    ProviderEventAction::Event(event) => yield event,
                    ProviderEventAction::ToolCall { call, next } => {
                        calls.push(call);
                        continuation = Some(next);
                    }
                    ProviderEventAction::Completed(event) => {
                        context.trace.observe_agent(&event).await;
                        yield event;
                        return;
                    }
                    ProviderEventAction::Failure(detail) => {
                        Err::<(), _>(agent_error(&provider_id, &detail))?;
                    }
                }
            }

            let Some(continuation) = continuation else {
                Err::<(), _>(agent_error(
                    &provider_id,
                    "provider ended an agent step without a completion or tool call",
                ))?;
                unreachable!();
            };

            loop_guard
                .accept_batch(&calls)
                .map_err(|error| loop_guard_error(&provider_id, error))?;

            for call in calls.iter().cloned() {
                let event = AgentRunEvent::ToolCall(call);
                context.trace.observe_agent(&event).await;
                yield event;
            }

            let mut outputs = Vec::with_capacity(calls.len());

            let mut tool_stream = tool::execute_tools(
                context.clone(),
                calls,
                approvals.clone(),
                provider_id.clone(),
                permissions.clone(),
            );
            while let Some(event) = tool_stream.next().await {
                let event = event?;
                if let AgentRunEvent::ToolResult(output) = &event {
                    outputs.push(output.clone());
                }

                let approval_event = matches!(&event, AgentRunEvent::ApprovalRequired(_));
                if approval_event {
                    yield event.clone();
                }
                context.trace.observe_agent(&event).await;
                if !approval_event {
                    yield event;
                }
            }

            provider_stream = context.provider.resume(AgentResumeRequest {
                continuation,
                outputs,
                instructions: request.instructions.clone(),
                tools: request.tools.clone(),
            });
        }
    })
}

enum ProviderEventAction {
    Ignore,
    Event(AgentRunEvent),
    ToolCall {
        call: AgentToolCall,
        next: AgentContinuation,
    },
    Completed(AgentRunEvent),
    Failure(String),
}

async fn observe_provider_event(
    context: &AgentStreamContext,
    event: AgentProviderEvent,
    first_started: &mut bool,
) -> ProviderEventAction {
    let event = match event {
        AgentProviderEvent::Started => {
            if *first_started {
                return ProviderEventAction::Ignore;
            }
            *first_started = true;
            AgentRunEvent::Started
        }
        AgentProviderEvent::TextDelta(delta) => AgentRunEvent::TextDelta(delta),
        AgentProviderEvent::TextDeltaWithPhase { delta, phase } => {
            AgentRunEvent::TextDeltaWithPhase { delta, phase }
        }
        AgentProviderEvent::ReasoningSummaryStarted { key, title } => {
            AgentRunEvent::ReasoningSummaryStarted { key, title }
        }
        AgentProviderEvent::ReasoningSummaryDelta { key, delta } => {
            AgentRunEvent::ReasoningSummaryDelta { key, delta }
        }
        AgentProviderEvent::ReasoningSummaryCompleted { key, text } => {
            AgentRunEvent::ReasoningSummaryCompleted { key, text }
        }
        AgentProviderEvent::ToolCall { call, continuation } => {
            return ProviderEventAction::ToolCall {
                call,
                next: continuation,
            };
        }
        AgentProviderEvent::Completed(outcome) => {
            if let Some(review) = &context.review
                && let Err(error) = review.finish().await
            {
                return ProviderEventAction::Failure(format!(
                    "could not finish AgentFS review session: {error}"
                ));
            }
            return ProviderEventAction::Completed(AgentRunEvent::Completed(outcome));
        }
    };

    context.trace.observe_agent(&event).await;
    ProviderEventAction::Event(event)
}

#[derive(Default)]
struct AgentLoopGuard {
    rounds: usize,
    tool_calls: usize,
    last_batch_fingerprint: Option<Vec<(String, String)>>,
    consecutive_identical_batches: usize,
}

impl AgentLoopGuard {
    fn accept_batch(&mut self, calls: &[AgentToolCall]) -> Result<(), AgentLoopFailure> {
        let observed_rounds = self.rounds.saturating_add(1);
        let observed_tool_calls = self.tool_calls.saturating_add(calls.len());

        if observed_rounds > MAX_AGENT_ROUNDS || observed_tool_calls > MAX_AGENT_TOOL_CALLS {
            return Err(AgentLoopFailure::Limit {
                observed_rounds,
                observed_tool_calls,
            });
        }

        let fingerprint = tool_batch_fingerprint(calls);

        if fingerprint.is_empty() {
            self.last_batch_fingerprint = None;
            self.consecutive_identical_batches = 0;
        } else if self.last_batch_fingerprint.as_ref() == Some(&fingerprint) {
            self.consecutive_identical_batches += 1;
        } else {
            self.last_batch_fingerprint = Some(fingerprint);
            self.consecutive_identical_batches = 1;
        }

        if self.consecutive_identical_batches >= 3 {
            return Err(AgentLoopFailure::Repeated);
        }

        self.rounds = observed_rounds;
        self.tool_calls = observed_tool_calls;

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AgentLoopFailure {
    Limit {
        observed_rounds: usize,
        observed_tool_calls: usize,
    },
    Repeated,
}

fn loop_guard_error(
    provider: &magenta_core::ProviderId,
    error: AgentLoopFailure,
) -> magenta_core::ProviderError {
    match error {
        AgentLoopFailure::Limit {
            observed_rounds,
            observed_tool_calls,
        } => magenta_core::ProviderError::with_kind_and_diagnostic(
            provider.clone(),
            magenta_core::ProviderErrorKind::AgentLimitReached,
            magenta_core::ProviderErrorDiagnostic::AgentLimits {
                observed_rounds,
                permitted_rounds: MAX_AGENT_ROUNDS,
                observed_tool_calls,
                permitted_tool_calls: MAX_AGENT_TOOL_CALLS,
            },
            std::io::Error::other(format!(
                concat!(
                    "agent loop limit exceeded: observed {} rounds and ",
                    "{} tool calls; permitted {} rounds and {} tool calls",
                ),
                observed_rounds, observed_tool_calls, MAX_AGENT_ROUNDS, MAX_AGENT_TOOL_CALLS,
            )),
        ),
        AgentLoopFailure::Repeated => magenta_core::ProviderError::with_kind(
            provider.clone(),
            magenta_core::ProviderErrorKind::RepeatedToolCalls,
            std::io::Error::other("repeated tool calls without progress"),
        ),
    }
}

fn tool_batch_fingerprint(calls: &[AgentToolCall]) -> Vec<(String, String)> {
    calls
        .iter()
        .map(|call| (call.name.clone(), canonical_arguments(&call.arguments)))
        .collect()
}

fn canonical_arguments(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments).map_or_else(
        |_| format!("raw:{arguments}"),
        |value| format!("json:{}", canonical_json(&value)),
    )
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).expect("JSON strings are serializable")
        }
        serde_json::Value::Array(values) => {
            let values = values.iter().map(canonical_json).collect::<Vec<_>>();

            format!("[{}]", values.join(","))
        }
        serde_json::Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(left, _)| *left);

            let entries = entries
                .into_iter()
                .map(|(key, value)| {
                    let key =
                        serde_json::to_string(key).expect("JSON object keys are serializable");
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>();

            format!("{{{}}}", entries.join(","))
        }
    }
}

#[cfg(test)]
#[path = "../../test/agent/stream.rs"]
mod tests;
