use super::*;

#[test]
fn generation_configuration_keeps_provider_model_and_effort_together() {
    let configuration = GenerationConfig::new(
        ProviderId::new("anthropic"),
        ModelId::new("sonnet"),
        EffortLevel::High,
    );

    assert_eq!(configuration.provider, ProviderId::new("anthropic"));
    assert_eq!(configuration.model, ModelId::new("sonnet"));
    assert_eq!(configuration.effort, EffortLevel::High);
    assert_eq!(configuration.limits, GenerationLimits::default());
}

#[test]
fn persisted_configuration_without_limits_uses_conservative_defaults() {
    let configuration: GenerationConfig = serde_json::from_value(serde_json::json!({
        "provider": "openai",
        "model": "legacy-model",
        "effort": "Medium"
    }))
    .expect("legacy configuration should deserialize");

    assert_eq!(configuration.limits, GenerationLimits::default());
}

#[test]
fn effort_levels_preserve_provider_specific_wire_values() {
    let values = [
        "none",
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "max",
        "thinking_budget",
    ];
    let efforts = values
        .iter()
        .map(|value| EffortLevel::from_wire(value).expect("effort should be valid"))
        .collect::<Vec<_>>();

    assert_eq!(efforts[0], EffortLevel::None);
    assert_eq!(efforts[1], EffortLevel::Minimal);
    assert_eq!(efforts[5], EffortLevel::XHigh);
    assert_eq!(efforts[6].label(), "Max");
    assert_eq!(efforts[7].wire_value(), "thinking_budget");
    assert_eq!(efforts[7].label(), "Thinking Budget");
}

#[test]
fn generation_outcome_keeps_finish_reason_and_usage_together() {
    let outcome = GenerationOutcome::new(
        FinishReason::Length,
        Some(TokenUsage {
            input_tokens: 21,
            output_tokens: 34,
        }),
    );

    assert_eq!(outcome.finish_reason, FinishReason::Length);
    assert_eq!(
        outcome.usage,
        Some(TokenUsage {
            input_tokens: 21,
            output_tokens: 34,
        })
    );
}
