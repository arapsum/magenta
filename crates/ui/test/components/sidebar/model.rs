use super::*;

#[test]
fn pinned_summaries_are_not_part_of_the_recency_groups() {
    let conversations = demo_conversations();
    assert_eq!(conversations.iter().filter(|item| item.pinned).count(), 2);
    assert!(
        conversations
            .iter()
            .filter(|item| item.pinned)
            .all(|item| item.period == ConversationPeriod::PreviousSevenDays)
    );
}

#[test]
fn search_matching_is_case_insensitive_and_handles_empty_queries() {
    assert!(title_matches("Streaming responses in GPUI", "gpui"));
    assert!(title_matches("Streaming responses in GPUI", ""));
    assert!(!title_matches("Streaming responses in GPUI", "sqlite"));
}
