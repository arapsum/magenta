use magenta_core::{
    ConversationMode, EffortLevel, GenerationConfig, GenerationSettings, ModelDescriptor, ModelId,
    ProviderId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationFallbackReason {
    ModelUnavailable {
        requested_provider: ProviderId,
        requested_model: ModelId,
        effective_provider: ProviderId,
        effective_model: ModelId,
    },
    ProviderUnavailable {
        requested_provider: ProviderId,
        requested_model: ModelId,
        effective_provider: ProviderId,
        effective_model: ModelId,
    },
    EffortUnavailable {
        provider: ProviderId,
        model: ModelId,
        requested: EffortLevel,
        effective: EffortLevel,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenerationResolution {
    pub config: Option<GenerationConfig>,
    pub fallback: Option<GenerationFallbackReason>,
}

impl GenerationResolution {
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            config: None,
            fallback: None,
        }
    }
}

/// Resolves a new-conversation generation configuration from preferences and a live catalog.
///
/// Existing conversation configurations must bypass this policy and remain authoritative.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_generation_defaults(
    mode: ConversationMode,
    settings: &GenerationSettings,
    models: &[ModelDescriptor],
) -> GenerationResolution {
    let Some(preference) = settings.for_mode(mode) else {
        return automatic_resolution(models);
    };

    let exact = models.iter().find(|model| {
        if model.provider != preference.provider {
            return false;
        }
        model.id == preference.model
    });
    let (model, fallback) = if let Some(model) = exact {
        (model, None)
    } else {
        let same_provider =
            highest_priority_model(models, |model| model.provider == preference.provider);
        if let Some(model) = same_provider {
            (
                model,
                Some(GenerationFallbackReason::ModelUnavailable {
                    requested_provider: preference.provider.clone(),
                    requested_model: preference.model.clone(),
                    effective_provider: model.provider.clone(),
                    effective_model: model.id.clone(),
                }),
            )
        } else {
            let Some(model) = highest_priority_model(models, |_| true) else {
                return GenerationResolution::unavailable();
            };
            (
                model,
                Some(GenerationFallbackReason::ProviderUnavailable {
                    requested_provider: preference.provider.clone(),
                    requested_model: preference.model.clone(),
                    effective_provider: model.provider.clone(),
                    effective_model: model.id.clone(),
                }),
            )
        }
    };

    let effort = if model.supported_efforts.contains(&preference.effort) {
        preference.effort.clone()
    } else {
        model.default_effort.clone()
    };
    let fallback = fallback.or_else(|| {
        (effort != preference.effort).then(|| GenerationFallbackReason::EffortUnavailable {
            provider: model.provider.clone(),
            model: model.id.clone(),
            requested: preference.effort.clone(),
            effective: effort.clone(),
        })
    });

    GenerationResolution {
        config: Some(
            GenerationConfig::new(model.provider.clone(), model.id.clone(), effort)
                .with_limits(model.limits),
        ),
        fallback,
    }
}

fn automatic_resolution(models: &[ModelDescriptor]) -> GenerationResolution {
    let Some(model) = highest_priority_model(models, |_| true) else {
        return GenerationResolution::unavailable();
    };

    GenerationResolution {
        config: Some(
            GenerationConfig::new(
                model.provider.clone(),
                model.id.clone(),
                model.default_effort.clone(),
            )
            .with_limits(model.limits),
        ),
        fallback: None,
    }
}

fn highest_priority_model(
    models: &[ModelDescriptor],
    predicate: impl Fn(&ModelDescriptor) -> bool,
) -> Option<&ModelDescriptor> {
    let mut selected = None;
    for model in models.iter().filter(|model| predicate(model)) {
        let should_replace = selected
            .as_ref()
            .is_none_or(|current: &&ModelDescriptor| model.priority > current.priority);
        if should_replace {
            selected = Some(model);
        }
    }
    selected
}

#[cfg(test)]
#[path = "../../test/generation_defaults.rs"]
mod tests;
