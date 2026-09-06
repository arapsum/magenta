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
    pub updated: String,
    pub period: ConversationPeriod,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    NewChat,
    OpenConversation(ConversationId),
    OpenSettings,
    BeginLogin,
    SignOut,
    ToggleTheme,
    SetPinned(ConversationId, bool),
    RetryHistory,
}

#[cfg(test)]
pub fn demo_conversations() -> Vec<ConversationSummary> {
    vec![
        ConversationSummary {
            id: ConversationId(1),
            title: "Designing Magenta's provider boundary".to_owned(),
            updated: "3d".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: true,
        },
        ConversationSummary {
            id: ConversationId(2),
            title: "Native Markdown rendering".to_owned(),
            updated: "5d".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: true,
        },
        ConversationSummary {
            id: ConversationId(3),
            title: "Reducing idle memory usage".to_owned(),
            updated: "2h".to_owned(),
            period: ConversationPeriod::Today,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(4),
            title: "Streaming responses in GPUI".to_owned(),
            updated: "5h".to_owned(),
            period: ConversationPeriod::Today,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(5),
            title: "SQLite conversation schema".to_owned(),
            updated: "1d".to_owned(),
            period: ConversationPeriod::Yesterday,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(6),
            title: "Keyboard shortcut map".to_owned(),
            updated: "1d".to_owned(),
            period: ConversationPeriod::Yesterday,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(7),
            title: "Cross-provider model mapping".to_owned(),
            updated: "3d".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(8),
            title: "Accessible code blocks".to_owned(),
            updated: "5d".to_owned(),
            period: ConversationPeriod::PreviousSevenDays,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(9),
            title: "Linux window integration".to_owned(),
            updated: "8d".to_owned(),
            period: ConversationPeriod::Older,
            pinned: false,
        },
        ConversationSummary {
            id: ConversationId(10),
            title: "Conversation persistence boundaries".to_owned(),
            updated: "12d".to_owned(),
            period: ConversationPeriod::Older,
            pinned: false,
        },
    ]
}

impl From<magenta_core::ConversationSummary> for ConversationSummary {
    fn from(summary: magenta_core::ConversationSummary) -> Self {
        let now = chrono::Local::now();
        let updated_at = chrono::DateTime::from_timestamp_millis(summary.updated_at.0)
            .map(|time| time.with_timezone(&chrono::Local));
        let days = updated_at.as_ref().map_or(i64::MAX, |updated_at| {
            now.date_naive()
                .signed_duration_since(updated_at.date_naive())
                .num_days()
        });
        let period = match days {
            ..=0 => ConversationPeriod::Today,
            1 => ConversationPeriod::Yesterday,
            2..=7 => ConversationPeriod::PreviousSevenDays,
            _ => ConversationPeriod::Older,
        };
        let elapsed_minutes = updated_at.as_ref().map_or(i64::MAX, |updated_at| {
            now.signed_duration_since(*updated_at).num_minutes().max(0)
        });
        let updated = match elapsed_minutes {
            0 => "now".to_owned(),
            1..=59 => format!("{elapsed_minutes}m"),
            60..=1439 => format!("{}h", elapsed_minutes / 60),
            _ => format!("{}d", elapsed_minutes / 1_440),
        };
        Self {
            id: summary.id,
            title: summary.title,
            updated,
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
