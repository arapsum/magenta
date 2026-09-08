use std::fs;

use futures_util::StreamExt as _;
use magenta_core::{
    WorkspaceCommand, WorkspaceCommandEvent, WorkspaceCommandRunner, WorkspaceCommandStatus,
};

use super::{BubblewrapCommandRunner, PreparedCommand};

#[test]
fn rejects_absolute_programs_parent_directories_and_excessive_timeouts() {
    let directory = tempfile::tempdir().unwrap();
    for command in [
        command("/usr/bin/true", 10),
        command("../true", 10),
        command("true", 601),
    ] {
        assert!(PreparedCommand::new(directory.path(), &command).is_err());
    }
}

#[test]
fn bubblewrap_confines_writes_and_masks_protected_files() {
    let Ok(runner) = BubblewrapCommandRunner::new() else {
        return;
    };
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(".env"), "SECRET=value").unwrap();
        let command = WorkspaceCommand {
            program: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "cat .env; printf done > generated.txt; printf warning >&2".to_owned(),
            ],
            cwd: ".".to_owned(),
            timeout_seconds: 10,
        };
        let events = runner
            .run(directory.path().to_path_buf(), command)
            .collect::<Vec<_>>()
            .await;
        let result = events
            .into_iter()
            .filter_map(Result::ok)
            .find_map(|event| match event {
                WorkspaceCommandEvent::Finished(result) => Some(result),
                WorkspaceCommandEvent::Output { .. } => None,
            })
            .expect("command should finish");
        assert_eq!(result.exit_code, Some(0), "{result:#?}");
        assert!(!result.stdout.contains("SECRET"));
        assert!(result.stderr.contains("warning"));
        assert_eq!(
            fs::read_to_string(directory.path().join("generated.txt")).unwrap(),
            "done"
        );
    });
}

#[test]
fn bubblewrap_enforces_timeouts_and_cancellation() {
    let Ok(runner) = BubblewrapCommandRunner::new() else {
        return;
    };
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let timed_out = WorkspaceCommand {
            program: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "sleep 2; printf late > timed-out.txt".to_owned(),
            ],
            cwd: ".".to_owned(),
            timeout_seconds: 1,
        };
        let result = runner
            .run(directory.path().to_path_buf(), timed_out)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .find_map(|event| match event {
                WorkspaceCommandEvent::Finished(result) => Some(result),
                WorkspaceCommandEvent::Output { .. } => None,
            })
            .expect("timed command should finish");
        assert_eq!(result.status, WorkspaceCommandStatus::TimedOut);
        assert!(!directory.path().join("timed-out.txt").exists());

        let cancelled = WorkspaceCommand {
            program: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "(sleep 1; printf late > cancelled.txt) & wait".to_owned(),
            ],
            cwd: ".".to_owned(),
            timeout_seconds: 10,
        };
        let stream = runner.run(directory.path().to_path_buf(), cancelled);
        smol::Timer::after(std::time::Duration::from_millis(100)).await;
        drop(stream);
        smol::Timer::after(std::time::Duration::from_millis(1_100)).await;
        assert!(!directory.path().join("cancelled.txt").exists());
    });
}

#[test]
fn bubblewrap_exposes_the_read_only_rust_toolchain() {
    let Ok(runner) = BubblewrapCommandRunner::new() else {
        return;
    };
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let command = WorkspaceCommand {
            program: "cargo".to_owned(),
            args: vec!["--version".to_owned()],
            cwd: ".".to_owned(),
            timeout_seconds: 10,
        };
        let result = runner
            .run(directory.path().to_path_buf(), command)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .find_map(|event| match event {
                WorkspaceCommandEvent::Finished(result) => Some(result),
                WorkspaceCommandEvent::Output { .. } => None,
            })
            .expect("cargo should finish");
        assert_eq!(result.exit_code, Some(0), "{result:#?}");
        assert!(result.stdout.starts_with("cargo "));
    });
}

#[test]
fn bubblewrap_exposes_user_installed_node_and_pnpm() {
    let Ok(runner) = BubblewrapCommandRunner::new() else {
        return;
    };
    if runner.node_home.is_none() || runner.pnpm_home.is_none() {
        return;
    }
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        for program in ["node", "npm", "pnpm"] {
            let command = WorkspaceCommand {
                program: program.to_owned(),
                args: vec!["--version".to_owned()],
                cwd: ".".to_owned(),
                timeout_seconds: 10,
            };
            let result = runner
                .run(directory.path().to_path_buf(), command)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .filter_map(Result::ok)
                .find_map(|event| match event {
                    WorkspaceCommandEvent::Finished(result) => Some(result),
                    WorkspaceCommandEvent::Output { .. } => None,
                })
                .expect("toolchain command should finish");
            assert_eq!(result.exit_code, Some(0), "{program}: {result:#?}");
            assert!(!result.stdout.trim().is_empty(), "{program}: {result:#?}");
        }
    });
}

#[test]
fn bubblewrap_uses_installed_pnpm_for_version_pinned_projects() {
    let Ok(runner) = BubblewrapCommandRunner::new() else {
        return;
    };
    if runner.pnpm_home.is_none() {
        return;
    }
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("package.json"),
            r#"{"name":"fixture","packageManager":"pnpm@9.15.5"}"#,
        )
        .unwrap();
        let command = WorkspaceCommand {
            program: "pnpm".to_owned(),
            args: vec!["install".to_owned(), "--ignore-scripts".to_owned()],
            cwd: ".".to_owned(),
            timeout_seconds: 10,
        };
        let result = runner
            .run(directory.path().to_path_buf(), command)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .find_map(|event| match event {
                WorkspaceCommandEvent::Finished(result) => Some(result),
                WorkspaceCommandEvent::Output { .. } => None,
            })
            .expect("pnpm should finish");
        assert_eq!(result.exit_code, Some(0), "{result:#?}");
        assert!(directory.path().join("pnpm-lock.yaml").is_file());
    });
}

fn command(program: &str, timeout_seconds: u16) -> WorkspaceCommand {
    WorkspaceCommand {
        program: program.to_owned(),
        args: Vec::new(),
        cwd: ".".to_owned(),
        timeout_seconds,
    }
}
