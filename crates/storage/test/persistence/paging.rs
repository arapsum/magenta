use super::*;

#[test]
fn pages_are_ordered_without_gaps_and_provider_context_is_independent() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteConversationStore::new(directory.path().join("history.sqlite3"));
        store.initialize().await.unwrap();
        let mut id = None;
        for index in 0..56 {
            let pending = store.begin_turn(input(id)).await.unwrap();
            id = Some(pending.conversation.id);
            assert_eq!(pending.context.len(), index * 2 + 1);
            let mut assistant = pending.assistant_message;
            assistant.status = MessageStatus::Complete;
            assistant.content = format!("answer {index}");
            store.finalize(assistant).await.unwrap();
        }
        let id = id.unwrap();
        let mut page = store.load(id).await.unwrap().page;
        assert_eq!(page.messages.len(), 50);
        let mut sequences = Vec::new();
        loop {
            assert!(
                page.messages
                    .windows(2)
                    .all(|pair| pair[0].sequence < pair[1].sequence)
            );
            sequences.extend(page.messages.iter().map(|item| item.sequence.0));
            if !page.has_older {
                break;
            }
            page = store.earlier(id, page.older_cursor.unwrap()).await.unwrap();
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (0..112).collect::<Vec<_>>());

        let around = store.load_around(id, MessageSequence(80)).await.unwrap();
        assert!(around.page.has_older);
        assert_eq!(around.page.messages.first().unwrap().sequence.0, 56);
        assert!(
            around
                .page
                .messages
                .iter()
                .any(|message| message.sequence.0 == 80)
        );
    });
}
