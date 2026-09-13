use std::{fs, process::Command};

use magenta_core::{RepositoryAccess, RepositoryChangeKind, RepositoryDiffArea};
use tempfile::TempDir;

use super::{LocalRepository, parse_status};

fn repository() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(args)
            .status()
            .unwrap()
    };
    assert!(run(&["init", "-q"]).success());
    assert!(run(&["config", "user.name", "Magenta Test"]).success());
    assert!(run(&["config", "user.email", "magenta@example.invalid"]).success());
    directory
}

#[test]
fn porcelain_status_tracks_staged_and_unstaged_changes() {
    let parsed = parse_status(b"## main\0MM src/main.rs\0?? new file.txt\0").unwrap();
    assert_eq!(parsed.branch.as_deref(), Some("main"));
    assert_eq!(
        parsed.changes[0].unstaged,
        Some(RepositoryChangeKind::Untracked)
    );
    assert_eq!(
        parsed.changes[1].staged,
        Some(RepositoryChangeKind::Modified)
    );
    assert_eq!(
        parsed.changes[1].unstaged,
        Some(RepositoryChangeKind::Modified)
    );
}

#[test]
fn porcelain_status_classifies_unmerged_paths_as_conflicts() {
    let status = parse_status(b"## main\0UU src/lib.rs\0").unwrap();

    assert_eq!(status.changes.len(), 1);
    assert_eq!(status.changes[0].staged, None);
    assert_eq!(
        status.changes[0].unstaged,
        Some(RepositoryChangeKind::Conflicted)
    );
}

#[test]
fn repository_round_trip_stages_diffs_and_commits() {
    smol::block_on(async {
        let directory = repository();
        fs::write(directory.path().join("hello.txt"), "hello\n").unwrap();
        let access = LocalRepository;
        let status = access.status(directory.path().to_path_buf()).await.unwrap();
        assert_eq!(
            status.changes[0].unstaged,
            Some(RepositoryChangeKind::Untracked)
        );
        let diff = access
            .diff(
                directory.path().to_path_buf(),
                "hello.txt".into(),
                RepositoryDiffArea::Unstaged,
            )
            .await
            .unwrap();
        assert!(diff.unified_diff.contains("+hello"));
        access
            .stage(directory.path().to_path_buf(), vec!["hello.txt".into()])
            .await
            .unwrap();
        let committed = access
            .commit(directory.path().to_path_buf(), "Add greeting".into())
            .await
            .unwrap();
        assert_eq!(committed.summary, "Add greeting");
        assert!(
            access
                .status(directory.path().to_path_buf())
                .await
                .unwrap()
                .changes
                .is_empty()
        );
    });
}
