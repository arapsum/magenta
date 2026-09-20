use magenta_core::{
    AgentToolPolicy, CommandCatalog, CommandDescriptor, CommandId, CommandPromptRequirement,
    ConversationMode, GenerationConfig, Message, MessageRole, estimate_text_tokens,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandResolution {
    pub command_id: Option<CommandId>,
    pub prompt: String,
    pub title_seed: String,
    pub instructions: Option<String>,
    pub tool_policy: AgentToolPolicy,
    pub request_overhead_tokens: u64,
}

#[must_use]
pub fn resolve_normal(prompt: &str) -> CommandResolution {
    CommandResolution {
        command_id: None,
        prompt: prompt.to_owned(),
        title_seed: prompt.to_owned(),
        instructions: None,
        tool_policy: AgentToolPolicy::Standard,
        request_overhead_tokens: 0,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommandResolutionError {
    #[error("the selected command is no longer available for this provider")]
    Unavailable { command_id: CommandId },
    #[error("the selected command is unavailable in this mode")]
    WrongMode {
        command_id: CommandId,
        mode: ConversationMode,
    },
    #[error("the {command_id:?} command requires a prompt or attachment subject")]
    MissingSubject { command_id: CommandId },
}

/// Resolves a command for one submission.
///
/// # Errors
///
/// Returns an error when the provider does not advertise the command, the
/// command does not support the active mode, or a required subject is absent.
pub fn resolve_submission(
    catalog: &dyn CommandCatalog,
    generation: &GenerationConfig,
    mode: &ConversationMode,
    command_id: Option<&CommandId>,
    prompt: &str,
    has_attachment_subject: bool,
) -> Result<CommandResolution, CommandResolutionError> {
    let Some(command_id) = command_id else {
        return Ok(resolve_normal(prompt));
    };

    let descriptor = find_descriptor(catalog, &generation.provider, command_id)?;
    validate_mode(&descriptor, mode)?;

    let prompt = prompt.trim();
    let used_fallback = prompt.is_empty() && !has_attachment_subject;
    let prompt = if used_fallback {
        match &descriptor.prompt_requirement {
            CommandPromptRequirement::Required => {
                return Err(CommandResolutionError::MissingSubject {
                    command_id: command_id.clone(),
                });
            }
            CommandPromptRequirement::Optional { fallback_prompt } => fallback_prompt.as_str(),
        }
    } else {
        prompt
    };

    let fallback_tokens = if used_fallback {
        estimate_text_tokens(prompt)
    } else {
        0
    };
    Ok(CommandResolution {
        command_id: Some(command_id.clone()),
        prompt: prompt.to_owned(),
        title_seed: if used_fallback || prompt.trim().is_empty() {
            format!("/{}", command_id.as_str())
        } else {
            prompt.to_owned()
        },
        instructions: Some(descriptor.response_instructions.clone()),
        tool_policy: descriptor.agent_tool_policy,
        request_overhead_tokens: estimate_text_tokens(&descriptor.response_instructions)
            .saturating_add(fallback_tokens),
    })
}

/// Resolves the persisted command associated with an existing response.
///
/// # Errors
///
/// Returns an error when the provider no longer advertises the command or the
/// persisted conversation mode is no longer supported.
pub fn resolve_persisted(
    catalog: &dyn CommandCatalog,
    generation: &GenerationConfig,
    mode: &ConversationMode,
    command_id: Option<&CommandId>,
) -> Result<Option<CommandResolution>, CommandResolutionError> {
    let Some(command_id) = command_id else {
        return Ok(None);
    };

    let descriptor = find_descriptor(catalog, &generation.provider, command_id)?;
    validate_mode(&descriptor, mode)?;

    let (prompt, fallback_tokens) = match descriptor.prompt_requirement {
        CommandPromptRequirement::Required => (String::new(), 0),
        CommandPromptRequirement::Optional { fallback_prompt } => {
            let tokens = estimate_text_tokens(&fallback_prompt);
            (fallback_prompt, tokens)
        }
    };

    Ok(Some(CommandResolution {
        command_id: Some(command_id.clone()),
        prompt,
        title_seed: format!("/{}", command_id.as_str()),
        instructions: Some(descriptor.response_instructions.clone()),
        tool_policy: descriptor.agent_tool_policy,
        request_overhead_tokens: estimate_text_tokens(&descriptor.response_instructions)
            .saturating_add(fallback_tokens),
    }))
}

#[must_use]
pub fn apply_provider_prompt(
    mut messages: Vec<Message>,
    resolution: Option<&CommandResolution>,
) -> Vec<Message> {
    let Some(resolution) = resolution else {
        return messages;
    };
    if resolution.command_id.is_none() || resolution.prompt.trim().is_empty() {
        return messages;
    }

    if let Some(user_message) = messages
        .iter_mut()
        .rev()
        .find(|message| message.role == MessageRole::User)
        && user_message.content.trim().is_empty()
    {
        user_message.content.clone_from(&resolution.prompt);
    }

    messages
}

fn find_descriptor(
    catalog: &dyn CommandCatalog,
    provider: &magenta_core::ProviderId,
    command_id: &CommandId,
) -> Result<CommandDescriptor, CommandResolutionError> {
    catalog
        .commands(provider)
        .into_iter()
        .find(|descriptor| descriptor.id == *command_id)
        .ok_or_else(|| CommandResolutionError::Unavailable {
            command_id: command_id.clone(),
        })
}

fn validate_mode(
    descriptor: &CommandDescriptor,
    mode: &ConversationMode,
) -> Result<(), CommandResolutionError> {
    if descriptor.supports_mode(mode) {
        return Ok(());
    }
    Err(CommandResolutionError::WrongMode {
        command_id: descriptor.id.clone(),
        mode: mode.clone(),
    })
}

#[cfg(test)]
#[path = "../../test/commands.rs"]
mod tests;
