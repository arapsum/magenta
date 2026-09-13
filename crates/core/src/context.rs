use crate::{AgentToolDefinition, GenerationLimits, Message, MessageRole};

const REQUEST_FRAMING_TOKENS: u64 = 256;
const MESSAGE_FRAMING_TOKENS: u64 = 12;
const IMAGE_TOKENS: u64 = 4_096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextBudgetReport {
    pub estimated_input_tokens: u64,
    pub input_budget_tokens: u64,
    pub omitted_messages: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the newest message exceeds the model context budget")]
pub struct ContextTooLarge;

#[must_use]
pub fn estimate_text_tokens(value: &str) -> u64 {
    u64::try_from(value.len()).unwrap_or(u64::MAX).div_ceil(3)
}

#[must_use]
pub fn estimate_agent_overhead(instructions: &str, tools: &[AgentToolDefinition]) -> u64 {
    tools
        .iter()
        .fold(estimate_text_tokens(instructions), |total, tool| {
            total
                .saturating_add(estimate_text_tokens(&tool.name))
                .saturating_add(estimate_text_tokens(&tool.description))
                .saturating_add(estimate_text_tokens(&tool.parameters.to_string()))
                .saturating_add(2)
        })
}

/// Selects the newest whole turns that fit within the model's input budget.
///
/// # Errors
/// Returns [`ContextTooLarge`] when the newest user turn cannot fit by itself.
pub fn select_context(
    messages: &[Message],
    limits: GenerationLimits,
    extra_overhead_tokens: u64,
) -> Result<(Vec<Message>, ContextBudgetReport), ContextTooLarge> {
    let input_budget_tokens = limits
        .context_window_tokens
        .saturating_sub(limits.max_output_tokens)
        .saturating_sub(limits.context_window_tokens / 20);
    let fixed = REQUEST_FRAMING_TOKENS.saturating_add(extra_overhead_tokens);

    let groups = turn_groups(messages);
    let Some(newest) = groups.last() else {
        return Err(ContextTooLarge);
    };
    let newest_cost = group_cost(&messages[newest.clone()]);
    if fixed.saturating_add(newest_cost) > input_budget_tokens {
        return Err(ContextTooLarge);
    }

    let mut start = newest.start;
    let mut estimated_input_tokens = fixed.saturating_add(newest_cost);
    for group in groups[..groups.len() - 1].iter().rev() {
        let cost = group_cost(&messages[group.clone()]);
        if estimated_input_tokens.saturating_add(cost) > input_budget_tokens {
            break;
        }
        estimated_input_tokens = estimated_input_tokens.saturating_add(cost);
        start = group.start;
    }

    let selected = messages[start..].to_vec();
    let omitted_messages = messages.len().saturating_sub(selected.len());
    Ok((
        selected,
        ContextBudgetReport {
            estimated_input_tokens,
            input_budget_tokens,
            omitted_messages,
        },
    ))
}

fn turn_groups(messages: &[Message]) -> Vec<std::ops::Range<usize>> {
    let starts = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| (message.role == MessageRole::User).then_some(index))
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| *start..starts.get(index + 1).copied().unwrap_or(messages.len()))
        .collect()
}

fn group_cost(messages: &[Message]) -> u64 {
    messages.iter().fold(0, |total, message| {
        let images = message
            .attachments
            .iter()
            .filter(|attachment| attachment.mime_type.starts_with("image/"))
            .count();
        total
            .saturating_add(MESSAGE_FRAMING_TOKENS)
            .saturating_add(estimate_text_tokens(&message.content))
            .saturating_add(
                u64::try_from(images)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(IMAGE_TOKENS),
            )
    })
}

#[cfg(test)]
#[path = "../test/context.rs"]
mod tests;
