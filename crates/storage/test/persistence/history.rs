use super::*;

#[test]
fn reopen_preserves_messages_configuration_metadata_and_pins() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        let mut assistant = pending.assistant_message;
        assistant.content = "# Answer\n**Unicode** λ\n```rust\nlet x = 1;\n```".into();
        assistant.status = MessageStatus::Complete;
        assistant.generation_outcome = Some(GenerationOutcome::new(
            FinishReason::Other("custom-finish".into()),
            Some(TokenUsage {
                input_tokens: 31,
                output_tokens: 42,
            }),
        ));
        store.finalize(assistant.clone()).await.unwrap();
        store.set_pinned(id, true).await.unwrap();
        drop(store);

        let reopened = SqliteConversationStore::new(path);
        reopened.initialize().await.unwrap();
        let summaries = reopened.summaries().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].preview,
            "Explain `λ`\n```rust\nfn main() {}\n```"
        );
        assert!(summaries[0].pinned);
        assert!(summaries[0].updated_at >= summaries[0].created_at);
        let loaded = reopened.load(id).await.unwrap();
        assert_eq!(loaded.conversation, pending.conversation);
        assert_eq!(loaded.page.messages.len(), 2);
        assert_eq!(loaded.page.messages[0].message, pending.user_message);
        assert_eq!(loaded.page.messages[1].message, assistant);
        assert_eq!(loaded.page.messages[0].sequence.0, 0);
        assert_eq!(loaded.page.messages[1].sequence.0, 1);
        assert!(!loaded.page.has_older);
        assert_eq!(loaded.page.messages[1].generation, input(None).generation);

        let continued = reopened.begin_turn(input(Some(id))).await.unwrap();
        assert_eq!(continued.context.len(), 3);
        assert_eq!(continued.context[1], assistant);
        assert!(continued.user_message.id.0 > assistant.id.0);
    });
}

#[test]
fn agent_finalization_marks_the_run_completed() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();

        let mut agent_input = input(None);
        agent_input.mode = ConversationMode::Agent;
        agent_input.workspace_root = Some(directory.path().to_path_buf());
        let pending = store.begin_turn(agent_input).await.unwrap();
        let assistant_id = pending.assistant_message.id.0;
        let mut assistant = pending.assistant_message;
        assistant.status = MessageStatus::Complete;
        assistant.content = "Created the project.".into();

        store.finalize(assistant).await.unwrap();

        let connection = rusqlite::Connection::open(path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM agent_runs WHERE assistant_message_id = ?1",
                [assistant_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
    });
}

#[test]
fn rename_persists_without_changing_conversation_recency() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();

        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        store.set_pinned(id, true).await.unwrap();
        let before = store.summaries().await.unwrap().remove(0);

        store
            .rename(id, "Renamed conversation".into())
            .await
            .unwrap();
        let renamed = store.summaries().await.unwrap().remove(0);
        assert_eq!(renamed.title, "Renamed conversation");
        assert!(renamed.pinned);
        assert_eq!(renamed.updated_at, before.updated_at);
        assert_eq!(
            store.load(id).await.unwrap().conversation.title,
            renamed.title
        );
        assert_eq!(
            store
                .rename(ConversationId(999), "Missing".into())
                .await
                .unwrap_err()
                .kind,
            StorageErrorKind::NotFound
        );

        drop(store);
        let reopened = SqliteConversationStore::new(path);
        reopened.initialize().await.unwrap();
        assert_eq!(
            reopened.load(id).await.unwrap().conversation.title,
            "Renamed conversation"
        );
    });
}

#[test]
fn full_text_search_tracks_titles_message_updates_renames_and_deletes() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path);
        store.initialize().await.unwrap();

        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        let title_match = store.search("unic".into(), 10).await.unwrap();
        assert_eq!(title_match.len(), 1);
        assert_eq!(title_match[0].conversation_id, id);
        assert!(title_match[0].message_id.is_none());
        assert_eq!(
            &title_match[0].title[title_match[0].title_highlights[0].clone()],
            "Unicode"
        );

        let body_match = store.search("main".into(), 10).await.unwrap();
        assert_eq!(body_match.len(), 1);
        assert_eq!(body_match[0].message_id, Some(pending.user_message.id));
        assert_eq!(body_match[0].message_sequence.unwrap().0, 0);
        assert!(body_match[0].snippet.contains("main"));
        assert!(!body_match[0].snippet_highlights.is_empty());

        let mut assistant = pending.assistant_message;
        assistant.content = "A distinctly searchable phosphorescent answer".into();
        assistant.status = MessageStatus::Complete;
        store.finalize(assistant.clone()).await.unwrap();
        let finalized = store.search("phosphor".into(), 10).await.unwrap();
        assert_eq!(finalized[0].message_id, Some(assistant.id));

        store.rename(id, "Retitled archive".into()).await.unwrap();
        assert!(store.search("unicode".into(), 10).await.unwrap().is_empty());
        assert_eq!(
            store.search("retit".into(), 10).await.unwrap()[0].conversation_id,
            id
        );

        store.delete(id).await.unwrap();
        assert!(
            store
                .search("phosphor".into(), 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(store.search("   ".into(), 10).await.unwrap().is_empty());
    });
}
