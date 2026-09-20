use super::{
    can_approve_for_run,
    commands::{normalize_command_cwd, parse_command},
    parse_operation, tool_definitions,
};
use futures_util::{StreamExt as _, stream};
use magenta_core::{
    AgentProvider, AgentProviderStream, AgentRequest, AgentResumeRequest, AgentToolCall,
    Conversation, ConversationId, ConversationMode, EffortLevel, GenerationConfig, ModelId,
    ProviderId, WorkspaceAccess, WorkspaceCommand, WorkspaceFuture, WorkspaceMutation,
    WorkspaceOperation, WorkspacePreview,
};
use std::{path::PathBuf, sync::Arc};

struct NoopAgentProvider;

impl AgentProvider for NoopAgentProvider {
    fn start(&self, _: AgentRequest) -> AgentProviderStream {
        Box::pin(stream::empty())
    }

    fn resume(&self, _: AgentResumeRequest) -> AgentProviderStream {
        Box::pin(stream::empty())
    }
}

struct PreviewWorkspace;

impl WorkspaceAccess for PreviewWorkspace {
    fn prepare(
        &self,
        _: PathBuf,
        operation: WorkspaceOperation,
        _: bool,
    ) -> WorkspaceFuture<WorkspacePreview> {
        Box::pin(async move {
            Ok(WorkspacePreview {
                path: operation.path().to_owned(),
                summary: "create a file".to_owned(),
                output: String::new(),
                diff: Some("+hello".to_owned()),
                protected: false,
                mutation: Some(WorkspaceMutation {
                    path: operation.path().to_owned(),
                    expected_digest: None,
                    replacement: b"hello".to_vec(),
                    creates_file: true,
                    creates_directory: false,
                }),
            })
        })
    }

    fn commit(&self, _: PathBuf, _: WorkspaceMutation) -> WorkspaceFuture<String> {
        Box::pin(async { Ok("staged".to_owned()) })
    }
}

fn agent_context(root: PathBuf) -> super::super::AgentStreamContext {
    let store = magenta_storage::SqliteConversationStore::new(root.join("trace.db"));

    super::super::AgentStreamContext {
        provider: Arc::new(NoopAgentProvider),
        workspace: Arc::new(PreviewWorkspace),
        command_runner: None,
        repository: None,
        tool_policy: magenta_core::AgentToolPolicy::Standard,
        root: root.clone(),
        conversation: Conversation {
            id: ConversationId(1),
            title: "Workspace test".to_owned(),
            generation: GenerationConfig::new(
                ProviderId::new("test"),
                ModelId::new("model"),
                EffortLevel::Medium,
            ),
            mode: ConversationMode::Agent,
            workspace_root: Some(root),
        },
        trace: crate::trace::AssistantTraceRecorder::new(
            Arc::new(store),
            magenta_core::MessageId(1),
            magenta_core::AssistantTrace::default(),
        ),
        review: None,
        context_services: None,
    }
}

#[test]
fn tool_definitions_expose_only_supported_tools() {
    let definitions = tool_definitions(true);

    assert_eq!(
        definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "list_files",
            "search_text",
            "search_code",
            "save_memory_candidate",
            "read_file",
            "apply_patch",
            "create_file",
            "create_directory",
            "run_command"
        ]
    );
    assert_eq!(
        definitions
            .iter()
            .filter(|definition| definition.mutating)
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "apply_patch",
            "create_file",
            "create_directory",
            "run_command"
        ]
    );
    assert!(
        definitions
            .iter()
            .find(|definition| definition.name == "read_file")
            .is_some_and(|definition| definition.protected_read)
    );
    assert!(
        definitions
            .iter()
            .find(|definition| definition.name == "list_files")
            .is_some_and(|definition| !definition.protected_read)
    );
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

fn preview(protected: bool) -> WorkspacePreview {
    WorkspacePreview {
        path: "src/lib.rs".to_owned(),
        summary: "update file".to_owned(),
        output: String::new(),
        diff: None,
        protected,
        mutation: None,
    }
}

#[test]
fn run_permission_is_limited_to_unprotected_file_edits() {
    assert!(can_approve_for_run(
        &WorkspaceOperation::ApplyPatch {
            path: "src/lib.rs".to_owned(),
            unified_diff: String::new(),
        },
        &preview(false),
    ));
    assert!(can_approve_for_run(
        &WorkspaceOperation::CreateFile {
            path: "src/new.rs".to_owned(),
            content: String::new(),
        },
        &preview(false),
    ));
    assert!(!can_approve_for_run(
        &WorkspaceOperation::ReadFile {
            path: "src/lib.rs".to_owned(),
            start_line: None,
            line_count: None,
        },
        &preview(false),
    ));
    assert!(!can_approve_for_run(
        &WorkspaceOperation::ApplyPatch {
            path: "src/lib.rs".to_owned(),
            unified_diff: String::new(),
        },
        &preview(true),
    ));
}

#[test]
fn workspace_mutations_emit_approval_before_waiting_for_a_decision() {
    let root = tempfile::tempdir().expect("temporary workspace should exist");
    let call = AgentToolCall {
        id: "create-1".to_owned(),
        name: "create_file".to_owned(),
        arguments: r#"{"path":"notes/todo.md","content":"hello"}"#.to_owned(),
    };
    let (_, approvals) = async_channel::unbounded();
    let permissions = Arc::new(std::sync::Mutex::new(super::AgentRunPermissions::default()));
    let mut events = super::workspace::execute_workspace_tool(
        agent_context(root.path().to_path_buf()),
        call,
        approvals,
        ProviderId::new("test"),
        permissions,
    );

    let first = smol::block_on(events.next()).expect("proposed change should be emitted");
    assert!(matches!(
        first.expect("workspace stream should succeed"),
        magenta_core::AgentRunEvent::WorkspaceChange(change)
            if change.state == magenta_core::WorkspaceChangeState::Proposed
    ));

    let second = smol::block_on(events.next()).expect("approval should be emitted");
    assert!(matches!(
        second.expect("workspace stream should succeed"),
        magenta_core::AgentRunEvent::ApprovalRequired(approval)
            if approval.request_id == "create-1-approval"
    ));
}

#[test]
fn create_directory_is_offered_and_parsed_without_command_execution() {
    assert!(
        tool_definitions(false)
            .iter()
            .any(|tool| tool.name == "create_directory" && tool.mutating)
    );
    let call = AgentToolCall {
        id: "directory-1".to_owned(),
        name: "create_directory".to_owned(),
        arguments: r#"{"path":"HelloExpress"}"#.to_owned(),
    };
    assert_eq!(
        parse_operation(&call),
        Ok(WorkspaceOperation::CreateDirectory {
            path: "HelloExpress".to_owned()
        })
    );
}
