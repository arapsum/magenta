use super::WorkspaceCommand;

#[test]
fn display_quotes_arguments_without_changing_execution_arguments() {
    let command = WorkspaceCommand {
        program: "cargo".to_owned(),
        args: vec!["test".to_owned(), "name with spaces".to_owned()],
        cwd: ".".to_owned(),
        timeout_seconds: 120,
    };

    assert_eq!(command.display(), "cargo test 'name with spaces'");
}
