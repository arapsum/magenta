use std::collections::HashMap;

use crate::{CommandResolutionError, resolve_submission};
use magenta_core::{
    AgentToolPolicy, CommandDescriptor, CommandId, CommandPromptRequirement, ConversationMode,
    EffortLevel, GenerationConfig, ModelId, ProviderId, StaticCommandCatalog,
};

fn catalog() -> StaticCommandCatalog {
    StaticCommandCatalog::new(HashMap::from([(
        ProviderId::new("test"),
        vec![CommandDescriptor {
            id: CommandId::new("review"),
            label: "Review".to_owned(),
            description: "Review changes".to_owned(),
            supported_modes: vec![ConversationMode::Agent],
            prompt_requirement: CommandPromptRequirement::Optional {
                fallback_prompt: "Review the current workspace changes".to_owned(),
            },
            response_instructions: "Findings first".to_owned(),
            agent_tool_policy: AgentToolPolicy::ReadOnly,
        }],
    )]))
}

fn generation() -> GenerationConfig {
    GenerationConfig::new(
        ProviderId::new("test"),
        ModelId::new("model"),
        EffortLevel::Low,
    )
}

#[test]
fn bare_optional_command_uses_provider_fallback() {
    let result = resolve_submission(
        &catalog(),
        &generation(),
        &ConversationMode::Agent,
        Some(&CommandId::new("review")),
        "",
        false,
    )
    .expect("review should resolve");

    assert_eq!(result.prompt, "Review the current workspace changes");
    assert_eq!(result.title_seed, "/review");
    assert_eq!(result.tool_policy, AgentToolPolicy::ReadOnly);
}

#[test]
fn command_mode_is_validated_before_prompt_resolution() {
    let error = resolve_submission(
        &catalog(),
        &generation(),
        &ConversationMode::Chat,
        Some(&CommandId::new("review")),
        "",
        false,
    )
    .expect_err("review should be Work-only");

    assert!(matches!(error, CommandResolutionError::WrongMode { .. }));
}
