use magenta_core::{
    ConversationMode, EffortLevel, GenerationLimits, GenerationPreference, GenerationSettings,
    ModelDescriptor, ModelId, ProviderId,
};

use super::*;

fn model(
    provider: &str,
    id: &str,
    priority: i32,
    default_effort: EffortLevel,
    supported_efforts: Vec<EffortLevel>,
    limits: GenerationLimits,
) -> ModelDescriptor {
    ModelDescriptor {
        provider: ProviderId::new(provider),
        id: ModelId::new(id),
        display_name: id.to_owned(),
        description: None,
        priority,
        default_effort,
        supported_efforts,
        limits,
    }
}

fn settings(preference: Option<GenerationPreference>) -> GenerationSettings {
    GenerationSettings {
        chat: preference,
        work: None,
    }
}

#[test]
fn mode_lookup_is_independent() {
    let settings = GenerationSettings {
        chat: Some(GenerationPreference::new(
            ProviderId::new("openai"),
            ModelId::new("chat-model"),
            EffortLevel::Medium,
        )),
        work: Some(GenerationPreference::new(
            ProviderId::new("openai"),
            ModelId::new("work-model"),
            EffortLevel::High,
        )),
    };

    assert_eq!(
        settings
            .for_mode(ConversationMode::Chat)
            .map(|preference| preference.model.clone()),
        Some(ModelId::new("chat-model"))
    );
    assert_eq!(
        settings
            .for_mode(ConversationMode::Agent)
            .map(|preference| preference.model.clone()),
        Some(ModelId::new("work-model"))
    );
}

#[test]
fn automatic_uses_highest_priority_and_live_limits() {
    let limits = GenerationLimits {
        context_window_tokens: 1_000,
        max_output_tokens: 200,
    };
    let models = vec![
        model(
            "openai",
            "lower",
            3,
            EffortLevel::Low,
            vec![EffortLevel::Low],
            GenerationLimits::default(),
        ),
        model(
            "anthropic",
            "higher",
            9,
            EffortLevel::High,
            vec![EffortLevel::High],
            limits,
        ),
    ];

    let resolution = resolve_generation_defaults(
        ConversationMode::Chat,
        &GenerationSettings::default(),
        &models,
    );
    let config = resolution.config.expect("automatic should resolve");
    assert_eq!(config.model, ModelId::new("higher"));
    assert_eq!(config.provider, ProviderId::new("anthropic"));
    assert_eq!(config.effort, EffortLevel::High);
    assert_eq!(config.limits, limits);
    assert_eq!(resolution.fallback, None);
}

#[test]
fn exact_configured_model_and_effort_are_preserved() {
    let selected = model(
        "openai",
        "selected",
        2,
        EffortLevel::Low,
        vec![EffortLevel::Low, EffortLevel::High],
        GenerationLimits::default(),
    );
    let settings = settings(Some(GenerationPreference::new(
        selected.provider.clone(),
        selected.id.clone(),
        EffortLevel::High,
    )));

    let resolution = resolve_generation_defaults(ConversationMode::Chat, &settings, &[selected]);
    let config = resolution.config.expect("configured model should resolve");
    assert_eq!(config.model, ModelId::new("selected"));
    assert_eq!(config.effort, EffortLevel::High);
    assert_eq!(resolution.fallback, None);
}

#[test]
fn unsupported_effort_uses_the_resolved_model_default() {
    let selected = model(
        "openai",
        "selected",
        2,
        EffortLevel::Medium,
        vec![EffortLevel::Medium],
        GenerationLimits::default(),
    );
    let settings = settings(Some(GenerationPreference::new(
        selected.provider.clone(),
        selected.id.clone(),
        EffortLevel::High,
    )));

    let resolution = resolve_generation_defaults(ConversationMode::Chat, &settings, &[selected]);
    assert_eq!(resolution.config.unwrap().effort, EffortLevel::Medium);
    assert!(matches!(
        resolution.fallback,
        Some(GenerationFallbackReason::EffortUnavailable { .. })
    ));
}

#[test]
fn missing_model_falls_back_within_provider_before_cross_provider() {
    let same_provider = model(
        "openai",
        "available-openai",
        4,
        EffortLevel::Medium,
        vec![EffortLevel::Medium],
        GenerationLimits::default(),
    );
    let other_provider = model(
        "anthropic",
        "available-anthropic",
        8,
        EffortLevel::High,
        vec![EffortLevel::High],
        GenerationLimits::default(),
    );
    let configured = GenerationPreference::new(
        ProviderId::new("openai"),
        ModelId::new("missing"),
        EffortLevel::Medium,
    );
    let resolution = resolve_generation_defaults(
        ConversationMode::Chat,
        &settings(Some(configured.clone())),
        &[same_provider.clone(), other_provider.clone()],
    );
    assert_eq!(
        resolution
            .config
            .as_ref()
            .map(|config| config.model.clone()),
        Some(same_provider.id)
    );
    assert!(matches!(
        resolution.fallback,
        Some(GenerationFallbackReason::ModelUnavailable { .. })
    ));

    let resolution = resolve_generation_defaults(
        ConversationMode::Chat,
        &settings(Some(configured)),
        &[other_provider],
    );
    assert_eq!(
        resolution
            .config
            .as_ref()
            .map(|config| config.model.clone()),
        Some(ModelId::new("available-anthropic"))
    );
    assert!(matches!(
        resolution.fallback,
        Some(GenerationFallbackReason::ProviderUnavailable { .. })
    ));
}

#[test]
fn empty_catalog_disables_resolution() {
    let resolution =
        resolve_generation_defaults(ConversationMode::Agent, &GenerationSettings::default(), &[]);
    assert_eq!(resolution, GenerationResolution::unavailable());
}
