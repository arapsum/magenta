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
    pub preview: String,
    pub updated: String,
    pub period: ConversationPeriod,
    pub pinned: bool,
    pub mode: magenta_core::ConversationMode,
    pub workspace_root: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    NewChat,
    OpenConversation(ConversationId),
    RenameConversation(ConversationId, String),
    OpenSettings,
    BeginLogin,
    SignOut,
    ToggleTheme,
    SetPinned(ConversationId, bool),
    DeleteConversation(ConversationId),
    RetryHistory,
    AddProject,
    ActivateProject(magenta_core::Project),
    ForgetProject(std::path::PathBuf),
}

#[cfg(test)]
pub fn demo_conversations() -> Vec<ConversationSummary> {
    [
        (1, "Designing Magenta's provider boundary", "3d", true),
        (2, "Native Markdown rendering", "5d", true),
        (3, "Reducing idle memory usage", "2h", false),
        (4, "Streaming responses in GPUI", "5h", false),
        (5, "SQLite conversation schema", "1d", false),
        (6, "Keyboard shortcut map", "1d", false),
        (7, "Cross-provider model mapping", "3d", false),
        (8, "Accessible code blocks", "5d", false),
        (9, "Linux window integration", "8d", false),
        (10, "Conversation persistence boundaries", "12d", false),
    ]
    .into_iter()
    .map(|(id, title, updated, pinned)| ConversationSummary {
        id: ConversationId(id),
        title: title.to_owned(),
        preview: String::new(),
        updated: updated.to_owned(),
        period: if updated.ends_with('h') {
            ConversationPeriod::Today
        } else {
            ConversationPeriod::PreviousSevenDays
        },
        pinned,
        mode: magenta_core::ConversationMode::Chat,
        workspace_root: None,
    })
    .collect()
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
            preview: summary
                .preview
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            updated,
            period,
            pinned: summary.pinned,
            mode: summary.mode,
            workspace_root: summary.workspace_root,
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
