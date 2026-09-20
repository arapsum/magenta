use std::collections::HashMap;

use crate::{ConversationMode, ProviderId};

/// Stable identifier for a provider-advertised slash command.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandId(pub String);

impl CommandId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().trim_start_matches('/').to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandPromptRequirement {
    Required,
    Optional { fallback_prompt: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentToolPolicy {
    Standard,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: CommandId,
    pub label: String,
    pub description: String,
    pub supported_modes: Vec<ConversationMode>,
    pub prompt_requirement: CommandPromptRequirement,
    pub response_instructions: String,
    pub agent_tool_policy: AgentToolPolicy,
}

impl CommandDescriptor {
    #[must_use]
    pub fn supports_mode(&self, mode: &ConversationMode) -> bool {
        self.supported_modes
            .iter()
            .any(|supported| supported == mode)
    }
}

/// Provider-neutral catalog access. Providers own the descriptor content;
/// application workflows only query the catalog through this port.
pub trait CommandCatalog: Send + Sync {
    fn commands(&self, provider: &ProviderId) -> Vec<CommandDescriptor>;
}

/// A small in-memory catalog useful to adapters and deterministic tests.
#[derive(Clone, Debug, Default)]
pub struct StaticCommandCatalog {
    catalogs: HashMap<ProviderId, Vec<CommandDescriptor>>,
}

impl StaticCommandCatalog {
    #[must_use]
    pub const fn new(catalogs: HashMap<ProviderId, Vec<CommandDescriptor>>) -> Self {
        Self { catalogs }
    }
}

impl CommandCatalog for StaticCommandCatalog {
    fn commands(&self, provider: &ProviderId) -> Vec<CommandDescriptor> {
        self.catalogs.get(provider).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "../../test/provider_commands.rs"]
mod tests;
