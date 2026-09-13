use magenta_core::AgentToolDefinition;

pub(super) fn tool_definitions(commands_available: bool) -> Vec<AgentToolDefinition> {
    let mut definitions = vec![
        definition(
            "list_files",
            "List workspace files and directories. Paths are relative; use '.' for the root.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "depth": {"type": "integer", "minimum": 0, "maximum": 6}
                },
                "required": ["path", "depth"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        definition(
            "search_text",
            "Find literal text in ignored-aware UTF-8 workspace files. Use '.' for the root.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "path": {"type": "string"},
                    "glob": {"type": ["string", "null"]}
                },
                "required": ["query", "path", "glob"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        definition(
            "read_file",
            "Read a bounded range from one UTF-8 workspace file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "start_line": {"type": ["integer", "null"]},
                    "line_count": {"type": ["integer", "null"]}
                },
                "required": ["path", "start_line", "line_count"],
                "additionalProperties": false
            }),
            false,
            true,
        ),
        definition(
            "apply_patch",
            "Propose a unified patch for one existing UTF-8 workspace file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "unified_diff": {"type": "string"}
                },
                "required": ["path", "unified_diff"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
        definition(
            "create_file",
            "Propose a new UTF-8 workspace file. It must not already exist.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
    ];
    if commands_available {
        definitions.push(definition(
            "run_command",
            "Run one non-interactive command in the selected workspace after explicit user approval. Arguments are passed directly without shell interpolation; network and stdin are unavailable.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "program": {"type": "string"},
                    "args": {"type": "array", "items": {"type": "string"}, "maxItems": 64},
                    "cwd": {"type": "string"},
                    "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600}
                },
                "required": ["program", "args", "cwd", "timeout_seconds"],
                "additionalProperties": false
            }),
            true,
            false,
        ));
    }
    definitions
}

fn definition(
    name: &str,
    description: &str,
    parameters: serde_json::Value,
    mutating: bool,
    protected_read: bool,
) -> AgentToolDefinition {
    AgentToolDefinition {
        name: name.to_owned(),
        description: description.to_owned(),
        parameters,
        mutating,
        protected_read,
    }
}
