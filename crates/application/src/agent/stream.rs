use std::sync::{Arc, Mutex};

use async_channel::Receiver;
use futures_util::StreamExt as _;
use magenta_core::{
    AgentProviderEvent, AgentRequest, AgentResumeRequest, AgentRunEvent, AgentRunStream,
    AgentToolCall,
};

use super::{
    AgentStreamContext, ApprovalResponse, MAX_AGENT_ROUNDS, MAX_AGENT_TOOL_CALLS, agent_error,
    tools,
};
use tools::AgentRunPermissions;

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
                match event? {
                    AgentProviderEvent::Started => {
                        if !first_started {
                            first_started = true;
                            yield AgentRunEvent::Started;
                        }
                    }
                    AgentProviderEvent::TextDelta(delta) => {
                        yield AgentRunEvent::TextDelta(delta);
                    }
                    AgentProviderEvent::ToolCall { call, continuation: next } => {
                        calls.push(call.clone());
                        continuation = Some(next);
                    }
                    AgentProviderEvent::Completed(outcome) => {
                        yield AgentRunEvent::Completed(outcome);
                        return;
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
                yield AgentRunEvent::ToolCall(call);
            }

            let mut outputs = Vec::with_capacity(calls.len());
            let mut tool_stream = tools::execute_tools(
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
                yield event;
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
                "agent loop limit exceeded: observed {observed_rounds} rounds and {observed_tool_calls} tool calls; permitted {MAX_AGENT_ROUNDS} rounds and {MAX_AGENT_TOOL_CALLS} tool calls"
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
mod tests {
    use super::*;

    fn call(id: &str, name: &str, arguments: &str) -> AgentToolCall {
        AgentToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    #[test]
    fn batch_fingerprints_ignore_call_ids_and_json_object_order() {
        let first = call("first", "read_file", r#"{"path":"src/lib.rs","start":1}"#);
        let second = call("second", "read_file", r#"{"start":1,"path":"src/lib.rs"}"#);
        assert_eq!(
            tool_batch_fingerprint(&[first]),
            tool_batch_fingerprint(&[second])
        );
    }

    #[test]
    fn invalid_arguments_use_the_raw_argument_string() {
        let first = call("first", "read_file", "not-json");
        let second = call("second", "read_file", " not-json");
        assert_ne!(
            tool_batch_fingerprint(&[first]),
            tool_batch_fingerprint(&[second])
        );
    }

    #[test]
    fn a_changed_batch_resets_repetition_tracking() {
        let mut guard = AgentLoopGuard::default();
        let first = call("first", "read_file", r#"{"path":"one"}"#);
        let changed = call("changed", "read_file", r#"{"path":"two"}"#);
        assert!(guard.accept_batch(std::slice::from_ref(&first)).is_ok());
        assert!(guard.accept_batch(std::slice::from_ref(&first)).is_ok());
        assert!(guard.accept_batch(std::slice::from_ref(&changed)).is_ok());
        assert!(guard.accept_batch(std::slice::from_ref(&changed)).is_ok());
        assert!(guard.accept_batch(std::slice::from_ref(&first)).is_ok());
        assert!(guard.accept_batch(std::slice::from_ref(&first)).is_ok());
    }

    #[test]
    fn the_third_identical_batch_is_rejected_before_execution() {
        let mut guard = AgentLoopGuard::default();
        let batch = [call("one", "read_file", r#"{"path":"same"}"#)];
        assert!(guard.accept_batch(&batch).is_ok());
        assert!(guard.accept_batch(&batch).is_ok());
        assert_eq!(guard.accept_batch(&batch), Err(AgentLoopFailure::Repeated));
    }

    #[test]
    fn the_guard_accepts_the_new_ceiling_then_reports_observed_counts() {
        let mut guard = AgentLoopGuard::default();
        for round in 0..MAX_AGENT_ROUNDS {
            let calls = (0..(MAX_AGENT_TOOL_CALLS / MAX_AGENT_ROUNDS))
                .map(|call_index| {
                    call(
                        &format!("{round}-{call_index}"),
                        "read_file",
                        &format!(r#"{{"path":"{round}/{call_index}"}}"#),
                    )
                })
                .collect::<Vec<_>>();
            assert!(guard.accept_batch(&calls).is_ok());
        }
        assert_eq!(guard.rounds, MAX_AGENT_ROUNDS);
        assert_eq!(guard.tool_calls, MAX_AGENT_TOOL_CALLS);

        let overflow = [call("overflow", "read_file", r#"{"path":"overflow"}"#)];
        let error = guard
            .accept_batch(&overflow)
            .expect_err("the next batch should exceed both limits");
        assert_eq!(
            error,
            AgentLoopFailure::Limit {
                observed_rounds: 65,
                observed_tool_calls: 257,
            }
        );
    }

    #[test]
    fn limit_failures_keep_the_out_of_limits_diagnostic_typed() {
        let error = loop_guard_error(
            &magenta_core::ProviderId::new("demo"),
            AgentLoopFailure::Limit {
                observed_rounds: 65,
                observed_tool_calls: 257,
            },
        );

        assert_eq!(
            error.kind,
            magenta_core::ProviderErrorKind::AgentLimitReached
        );
        assert_eq!(
            error.diagnostic,
            Some(magenta_core::ProviderErrorDiagnostic::AgentLimits {
                observed_rounds: 65,
                permitted_rounds: MAX_AGENT_ROUNDS,
                observed_tool_calls: 257,
                permitted_tool_calls: MAX_AGENT_TOOL_CALLS,
            })
        );
    }
}
