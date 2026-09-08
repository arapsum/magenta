use super::{
    commands::{normalize_command_cwd, parse_command},
    tools::{parse_operation, tool_definitions},
};
use magenta_core::{AgentToolCall, WorkspaceCommand, WorkspaceOperation};

#[test]
fn tool_definitions_expose_only_the_supported_workspace_operations() {
    let definitions = tool_definitions(true);

    assert_eq!(
        definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "list_files",
            "search_text",
            "read_file",
            "apply_patch",
            "create_file",
            "run_command"
        ]
    );
    assert_eq!(
        definitions
            .iter()
            .filter(|definition| definition.mutating)
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        vec!["apply_patch", "create_file", "run_command"]
    );
    assert!(definitions[2].protected_read);
    assert!(!definitions[0].protected_read);
}

#[test]
fn command_tool_is_omitted_without_a_sandbox_and_parses_structured_arguments() {
    assert!(
        !tool_definitions(false)
            .iter()
            .any(|tool| tool.name == "run_command")
    );
    let call = AgentToolCall {
        id: "command-1".to_owned(),
        name: "run_command".to_owned(),
        arguments:
            r#"{"program":"cargo","args":["test","--workspace"],"cwd":".","timeout_seconds":120}"#
                .to_owned(),
    };
    assert_eq!(
        parse_command(&call),
        Ok(WorkspaceCommand {
            program: "cargo".to_owned(),
            args: vec!["test".to_owned(), "--workspace".to_owned()],
            cwd: ".".to_owned(),
            timeout_seconds: 120,
        })
    );
}

#[test]
fn absolute_command_cwd_is_normalized_only_when_it_is_inside_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    let command = WorkspaceCommand {
        program: "cargo".to_owned(),
        args: Vec::new(),
        cwd: root.path().join("nested").to_string_lossy().into_owned(),
        timeout_seconds: 30,
    };
    assert_eq!(
        normalize_command_cwd(root.path(), command).unwrap().cwd,
        "nested"
    );

    let outside = tempfile::tempdir().unwrap();
    let command = WorkspaceCommand {
        program: "cargo".to_owned(),
        args: Vec::new(),
        cwd: outside.path().to_string_lossy().into_owned(),
        timeout_seconds: 30,
    };
    assert!(normalize_command_cwd(root.path(), command).is_err());
}

#[test]
fn parses_nullable_read_ranges_and_rejects_unknown_tools() {
    let call = AgentToolCall {
        id: "call-1".to_owned(),
        name: "read_file".to_owned(),
        arguments: r#"{"path":"src/lib.rs","start_line":null,"line_count":null}"#.to_owned(),
    };

    assert_eq!(
        parse_operation(&call),
        Ok(WorkspaceOperation::ReadFile {
            path: "src/lib.rs".to_owned(),
            start_line: None,
            line_count: None,
        })
    );

    let unknown = AgentToolCall {
        name: "run_shell".to_owned(),
        ..call
    };
    assert_eq!(
        parse_operation(&unknown),
        Err("unknown workspace tool run_shell".to_owned())
    );
}

#[test]
fn parses_mutating_arguments_without_widening_the_workspace_boundary() {
    let call = AgentToolCall {
        id: "call-2".to_owned(),
        name: "create_file".to_owned(),
        arguments: r#"{"path":"notes/todo.md","content":"- inspect"}"#.to_owned(),
    };

    assert_eq!(
        parse_operation(&call),
        Ok(WorkspaceOperation::CreateFile {
            path: "notes/todo.md".to_owned(),
            content: "- inspect".to_owned(),
        })
    );
}

#[test]
fn an_empty_listing_path_means_the_workspace_root() {
    let call = AgentToolCall {
        id: "call-3".to_owned(),
        name: "list_files".to_owned(),
        arguments: r#"{"path":"","depth":2}"#.to_owned(),
    };

    assert_eq!(
        parse_operation(&call),
        Ok(WorkspaceOperation::ListFiles {
            path: ".".to_owned(),
            depth: 2,
        })
    );
}
