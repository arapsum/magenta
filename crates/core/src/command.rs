use std::{error::Error, path::PathBuf, pin::Pin};

use futures_core::Stream;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub timeout_seconds: u16,
}

impl WorkspaceCommand {
    #[must_use]
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_+-./:=@,".contains(&byte))
    {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceCommandOutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceCommandStatus {
    Exited,
    TimedOut,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCommandResult {
    pub status: WorkspaceCommandStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceCommandEvent {
    Output {
        stream: WorkspaceCommandOutputStream,
        chunk: String,
    },
    Finished(WorkspaceCommandResult),
}

#[derive(Debug, thiserror::Error)]
#[error("workspace command failed")]
pub struct WorkspaceCommandError {
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl WorkspaceCommandError {
    #[must_use]
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

pub type WorkspaceCommandStream = Pin<
    Box<dyn Stream<Item = Result<WorkspaceCommandEvent, WorkspaceCommandError>> + Send + 'static>,
>;

pub trait WorkspaceCommandRunner: Send + Sync {
    fn run(&self, root: PathBuf, command: WorkspaceCommand) -> WorkspaceCommandStream;
}

#[cfg(test)]
mod tests {
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
}
