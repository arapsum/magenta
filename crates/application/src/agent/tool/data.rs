use super::{
    AgentContextServices, AgentStreamContext, AgentToolCall, AgentToolOutput, MemoryKind,
    MemoryState, NewAgentMemory,
};

pub(super) async fn execute_data_tool(
    context: &AgentStreamContext,
    call: &AgentToolCall,
) -> AgentToolOutput {
    execute_agent_data_tool(context, call).await
}

pub(super) fn compact_content(content: &str) -> String {
    const LIMIT: usize = 4_000;

    if content.len() <= LIMIT {
        return content.to_owned();
    }

    let mut end = LIMIT;
    while !content.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n… cached content truncated; use a narrower line range for full detail",
        &content[..end]
    )
}

async fn execute_agent_data_tool(
    context: &AgentStreamContext,
    call: &AgentToolCall,
) -> AgentToolOutput {
    let result = execute_agent_data_tool_result(context, call).await;

    match result {
        Ok(output) => AgentToolOutput {
            call_id: call.id.clone(),
            output,
            is_error: false,
        },
        Err(output) => AgentToolOutput {
            call_id: call.id.clone(),
            output,
            is_error: true,
        },
    }
}

async fn execute_agent_data_tool_result(
    context: &AgentStreamContext,
    call: &AgentToolCall,
) -> Result<String, String> {
    let services = context
        .context_services
        .as_ref()
        .ok_or_else(|| "agent data services are unavailable".to_owned())?;
    let arguments: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("invalid tool arguments: {error}"))?;

    match call.name.as_str() {
        "search_code" => search_code(context, services, &arguments).await,
        "save_memory_candidate" => save_memory_candidate(context, services, &arguments).await,
        _ => Err("unknown agent data tool".to_owned()),
    }
}

async fn search_code(
    context: &AgentStreamContext,
    services: &AgentContextServices,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    let query = arguments
        .get("query")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "query must be a non-empty string".to_owned())?;
    let embedding = services
        .embeddings
        .embed(vec![query.to_owned()])
        .await
        .ok()
        .and_then(|mut values| values.pop());
    let matches = services
        .code
        .search_code(
            context.root.clone(),
            query.to_owned(),
            embedding,
            context
                .review
                .as_ref()
                .map(|review| review.session_id.clone()),
            12,
        )
        .await
        .map_err(|error| error.to_string())?;

    Ok(matches
        .into_iter()
        .map(|item| {
            format!(
                "{}:{}-{} ({:.3})\n{}",
                item.chunk.path,
                item.chunk.start_line,
                item.chunk.end_line,
                item.score,
                item.chunk.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

async fn save_memory_candidate(
    context: &AgentStreamContext,
    services: &AgentContextServices,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    let memory_text = arguments
        .get("content")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "content must be a non-empty string".to_owned())?;
    let kind = match arguments.get("kind").and_then(serde_json::Value::as_str) {
        Some("fact") => MemoryKind::Fact,
        Some("preference") => MemoryKind::Preference,
        Some("decision") => MemoryKind::Decision,
        Some("procedure") => MemoryKind::Procedure,
        _ => return Err("unknown memory kind".to_owned()),
    };
    let confidence = arguments
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let embedding = services
        .embeddings
        .embed(vec![memory_text.to_owned()])
        .await
        .ok()
        .and_then(|mut values| values.pop());
    let memory = services
        .memories
        .remember(
            context.root.clone(),
            NewAgentMemory {
                kind,
                state: MemoryState::Candidate,
                content: memory_text.to_owned(),
                source_conversation_id: Some(context.conversation.id),
                confidence,
                embedding,
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    Ok(format!("saved memory candidate {} for review", memory.id))
}
