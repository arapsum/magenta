use crate::{
    AgentToolPolicy, CommandDescriptor, CommandId, CommandPromptRequirement, ConversationMode,
};

#[test]
fn command_ids_are_stored_without_a_leading_slash() {
    assert_eq!(CommandId::new("/review").as_str(), "review");
}

#[test]
fn descriptors_match_modes_and_requirements_explicitly() {
    let descriptor = CommandDescriptor {
        id: CommandId::new("review"),
        label: "Review".to_owned(),
        description: "Review changes".to_owned(),
        supported_modes: vec![ConversationMode::Agent],
        prompt_requirement: CommandPromptRequirement::Optional {
            fallback_prompt: "Review the current workspace changes".to_owned(),
        },
        response_instructions: "Use Markdown".to_owned(),
        agent_tool_policy: AgentToolPolicy::ReadOnly,
    };

    assert!(descriptor.supports_mode(&ConversationMode::Agent));
    assert!(!descriptor.supports_mode(&ConversationMode::Chat));
}
