use magenta_core::{
    AgentToolPolicy, CommandCatalog, CommandDescriptor, CommandId, CommandPromptRequirement,
    ConversationMode, ProviderId,
};

use super::OpenAiProvider;

const OPENAI: &str = "openai";

impl CommandCatalog for OpenAiProvider {
    fn commands(&self, provider: &ProviderId) -> Vec<CommandDescriptor> {
        if provider.0 != OPENAI {
            return Vec::new();
        }
        catalog()
    }
}

fn catalog() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor {
            id: CommandId::new("plan"),
            label: "Plan".to_owned(),
            description: "Create a structured implementation plan for the workspace.".to_owned(),
            supported_modes: vec![ConversationMode::Agent],
            prompt_requirement: CommandPromptRequirement::Required,
            response_instructions: concat!(
                "Write a readable Markdown implementation plan with these sections: ",
                "goal, relevant current state, implementation changes, interfaces, tests, ",
                "and assumptions. Inspect the workspace before making claims."
            )
            .to_owned(),
            agent_tool_policy: AgentToolPolicy::ReadOnly,
        },
        CommandDescriptor {
            id: CommandId::new("review"),
            label: "Review".to_owned(),
            description: "Review current workspace changes and report actionable findings.".to_owned(),
            supported_modes: vec![ConversationMode::Agent],
            prompt_requirement: CommandPromptRequirement::Optional {
                fallback_prompt: "Review the current workspace changes".to_owned(),
            },
            response_instructions: concat!(
                "Write a readable Markdown review. List findings before the summary, ordered ",
                "by severity, and include file/line evidence and remediation for each. ",
                "Explicitly state when no findings exist. Follow with test gaps and a short summary."
            )
            .to_owned(),
            agent_tool_policy: AgentToolPolicy::ReadOnly,
        },
        CommandDescriptor {
            id: CommandId::new("explain"),
            label: "Explain".to_owned(),
            description: "Explain a subject using the relevant workspace context.".to_owned(),
            supported_modes: vec![ConversationMode::Chat, ConversationMode::Agent],
            prompt_requirement: CommandPromptRequirement::Required,
            response_instructions: concat!(
                "Write a readable Markdown explanation with an overview, execution or data ",
                "flow, relevant components or examples, and caveats where useful."
            )
            .to_owned(),
            agent_tool_policy: AgentToolPolicy::ReadOnly,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_the_three_supported_commands() {
        let commands = catalog();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].id, CommandId::new("plan"));
        assert_eq!(commands[1].id, CommandId::new("review"));
        assert_eq!(commands[2].id, CommandId::new("explain"));
        assert!(commands.iter().all(|command| {
            command.agent_tool_policy == AgentToolPolicy::ReadOnly
                && !command.response_instructions.is_empty()
        }));
    }
}
