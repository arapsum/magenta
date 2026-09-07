use gpui::{Entity, SharedString};
use gpui_component::input::InputState;

use super::ConversationId;

pub(super) struct RenameState {
    pub(super) id: ConversationId,
    pub(super) original: String,
    pub(super) input: Entity<InputState>,
    pub(super) invalid: bool,
    pub(super) saving: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum HistoryActionState {
    #[default]
    Disabled,
    Enabled,
}

impl HistoryActionState {
    pub(super) const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

pub(super) struct ConversationActionData {
    pub(super) id: ConversationId,
    pub(super) selected: bool,
    pub(super) metadata: Option<String>,
    pub(super) group_name: SharedString,
    pub(super) pinned: bool,
    pub(super) history_actions: HistoryActionState,
}
