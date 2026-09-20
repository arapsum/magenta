use magenta_core::{AgentToolDefinition, AgentToolPolicy};

#[cfg(test)]
pub(super) fn tool_definitions(commands_available: bool) -> Vec<AgentToolDefinition> {
    tool_definitions_with_policy(commands_available, false, AgentToolPolicy::Standard)
}

pub(super) fn tool_definitions_with_policy(
    commands_available: bool,
    repository_available: bool,
    policy: AgentToolPolicy,
) -> Vec<AgentToolDefinition> {
    let mut definitions = vec![list_files(), search_text(), search_code()];
    if policy == AgentToolPolicy::Standard {
        definitions.push(save_memory_candidate());
    }
    definitions.push(read_file());
    if policy == AgentToolPolicy::Standard {
        definitions.push(apply_patch());
        definitions.push(create_file());
        definitions.push(create_directory());
        if commands_available {
            definitions.push(run_command());
        }
    }
    if repository_available {
        definitions.push(repository_status());
        definitions.push(repository_diff());
    }
    definitions
}

fn repository_status() -> AgentToolDefinition {
    definition(
        "repository_status",
        "Read the current branch and changed paths from the selected repository.",
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        false,
        false,
    )
}

fn repository_diff() -> AgentToolDefinition {
    definition(
        "repository_diff",
        "Read a staged or unstaged unified diff for one repository path.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "area": {"type": "string", "enum": ["staged", "unstaged"]}
            },
            "required": ["path", "area"],
            "additionalProperties": false
        }),
        false,
        true,
    )
}

fn list_files() -> AgentToolDefinition {
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
    )
}

fn search_text() -> AgentToolDefinition {
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
    )
}

fn search_code() -> AgentToolDefinition {
    definition(
        "search_code",
        "Search the project code index by meaning and text. Prefer this before broad literal searches.",
        serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": false
        }),
        false,
        false,
    )
}

fn save_memory_candidate() -> AgentToolDefinition {
    definition(
        "save_memory_candidate",
        "Save a project fact, preference, decision, or procedure as a candidate for user review.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {"type": "string"},
                "kind": {"type": "string", "enum": ["fact", "preference", "decision", "procedure"]},
                "confidence": {"type": "number", "minimum": 0, "maximum": 1}
            },
            "required": ["content", "kind", "confidence"],
            "additionalProperties": false
        }),
        false,
        false,
    )
}

fn read_file() -> AgentToolDefinition {
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
    )
}

fn apply_patch() -> AgentToolDefinition {
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
    )
}

fn create_file() -> AgentToolDefinition {
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
    )
}

fn create_directory() -> AgentToolDefinition {
    definition(
        "create_directory",
        "Propose a new directory in the selected workspace, including missing parent directories. It must not already exist.",
        serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": false
        }),
        true,
        false,
    )
}

fn run_command() -> AgentToolDefinition {
    definition(
        "run_command",
        concat!(
            "Run one non-interactive command in the selected workspace after explicit ",
            "user approval. Arguments are passed directly without shell interpolation; ",
            "network and stdin are unavailable.",
        ),
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
    )
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
