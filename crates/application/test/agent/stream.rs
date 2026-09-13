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
