pub use magenta_core::ConversationId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConversationPeriod {
    Today,
    Yesterday,
    PreviousSevenDays,
    Older,
}

impl ConversationPeriod {
    pub const ALL: [Self; 4] = [
        Self::Today,
        Self::Yesterday,
        Self::PreviousSevenDays,
        Self::Older,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::PreviousSevenDays => "Previous 7 days",
            Self::Older => "Older",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub title: String,
    pub period: ConversationPeriod,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    NewChat,
    OpenConversation(ConversationId),
    OpenSettings,
    SetPinned(ConversationId, bool),
    RetryHistory,
}

#[cfg(test)]
pub fn demo_conversations() -> Vec<ConversationSummary> {
    vec![
        ConversationSummary {
            id: ConversationId(1),
            title: "Designing Magenta's provider boundary".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: true,
        },
        ConversationSummary {
            id: ConversationId(2),
            title: "Native Markdown rendering".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: true,
        },
        ConversationSummary {
            id: ConversationId(3),
            title: "Reducing idle memory usage".to_owned(),
            period: ConversationPeriod::Today,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(4),
            title: "Streaming responses in GPUI".to_owned(),
            period: ConversationPeriod::Today,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(5),
            title: "SQLite conversation schema".to_owned(),
            period: ConversationPeriod::Yesterday,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(6),
            title: "Keyboard shortcut map".to_owned(),
            period: ConversationPeriod::Yesterday,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(7),
            title: "Cross-provider model mapping".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(8),
            title: "Accessible code blocks".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(9),
            title: "Linux window integration".to_owned(),
            period: ConversationPeriod::Older,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(10),
            title: "Conversation persistence boundaries".to_owned(),
            period: ConversationPeriod::Older,
            pinned: false,
        },
    ]
}

impl From<magenta_core::ConversationSummary> for ConversationSummary {
    fn from(summary: magenta_core::ConversationSummary) -> Self {
        let today = chrono::Local::now().date_naive();
        let date = chrono::DateTime::from_timestamp_millis(summary.updated_at.0)
            .map(|time| time.with_timezone(&chrono::Local).date_naive());
        let days = date.map_or(i64::MAX, |date| {
            today.signed_duration_since(date).num_days()
        });
        let period = match days {
            ..=0 => ConversationPeriod::Today,
            1 => ConversationPeriod::Yesterday,
            2..=7 => ConversationPeriod::PreviousSevenDays,
            _ => ConversationPeriod::Older,
        };
        Self {
            id: summary.id,
            title: summary.title,
            period,
            pinned: summary.pinned,
        }
    }
}

pub(super) fn title_matches(title: &str, search_term: &str) -> bool {
    let search_term = search_term.trim();
    search_term.is_empty() || title.to_lowercase().contains(&search_term.to_lowercase())
}

#[cfg(test)]
mod tests {
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
}
