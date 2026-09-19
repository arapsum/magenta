use super::*;
use magenta_core::{WorkspaceBrowser, WorkspaceEntryKind, WorkspaceOperation};

#[test]
fn creating_a_file_can_materialize_missing_parent_directories() {
    let directory = tempfile::tempdir().expect("workspace should exist");
    let operation = WorkspaceOperation::CreateFile {
        path: "hello-gtk/main.c".to_owned(),
        content: "int main(void) { return 0; }\n".to_owned(),
    };
    let preview = prepare(directory.path(), &operation, false).expect("preview should work");
    let mutation = preview.mutation.expect("create should produce a mutation");

    assert_eq!(
        commit(directory.path(), &mutation).expect("file should be created"),
        "created hello-gtk/main.c"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("hello-gtk/main.c"))
            .expect("created file should be readable"),
        "int main(void) { return 0; }\n"
    );
}

#[test]
fn creating_an_empty_directory_is_a_reviewable_mutation() {
    let workspace = tempfile::tempdir().unwrap();
    let operation = WorkspaceOperation::CreateDirectory {
        path: "HelloExpress/src".to_owned(),
    };
    let preview = prepare(workspace.path(), &operation, false).unwrap();
    assert!(
        preview
            .mutation
            .as_ref()
            .is_some_and(|mutation| mutation.creates_directory)
    );
    assert!(!workspace.path().join("HelloExpress").exists());
    let mutation = preview.mutation.unwrap();
    assert_eq!(
        commit(workspace.path(), &mutation).unwrap(),
        "created directory HelloExpress/src"
    );
    assert!(workspace.path().join("HelloExpress/src").is_dir());
    assert!(prepare(workspace.path(), &operation, false).is_err());
}

#[test]
fn browser_lists_direct_children_and_reads_supported_documents() {
    smol::block_on(async {
        let directory = tempfile::tempdir().expect("workspace should exist");
        fs::create_dir(directory.path().join("src")).expect("directory should be created");
        fs::create_dir(directory.path().join(".git")).expect("git directory should exist");
        fs::write(directory.path().join("src/main.rs"), "fn main() {}\n")
            .expect("source should be written");
        fs::write(directory.path().join("README.md"), "# Workspace\n")
            .expect("readme should be written");

        let workspace = LocalWorkspace;
        let entries = workspace
            .list_directory(directory.path().to_path_buf(), String::new())
            .await
            .expect("root should be browsable");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, WorkspaceEntryKind::Directory);
        assert_eq!(entries[0].path, "src");
        assert!(entries.iter().all(|entry| entry.name != ".git"));

        let document = workspace
            .read_document(directory.path().to_path_buf(), "src/main.rs".to_owned())
            .await
            .expect("source should be readable");
        assert_eq!(document.language, "rust");
        assert_eq!(document.content, "fn main() {}\n");
    });
}
