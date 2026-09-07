mod account_dropdown;
mod account_menu;
mod model;
mod projects;
mod rendering;
mod state;
#[cfg(test)]
mod tests;
use state::{ConversationActionData, HistoryActionState, RenameState};

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    ParentElement as _, Render, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, deferred, div, prelude::FluentBuilder as _, px,
};
use gpui_base::{Align, Placement, PopoverState, Positioner, actions::Cancel};
use gpui_component::{
    ActiveTheme as _, Collapsible, Disableable as _, ElementExt as _, Icon, IconName,
    Selectable as _, Sizable as _, StyledExt as _, ThemeStyled as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::{Sidebar, SidebarCollapsible, SidebarItem},
    v_flex,
};
use magenta_core::ProviderAccount;

use crate::app::OpenConversationFinder;

pub use model::{ConversationId, ConversationPeriod, ConversationSummary, SidebarEvent};

use self::model::title_matches;

const EXPANDED_WIDTH: gpui::Pixels = px(260.);
const ROW_HEIGHT: gpui::Pixels = px(30.);
const INITIAL_RECENCY_LIMIT: usize = 6;
const RECENCY_PAGE_SIZE: usize = 6;
const ACCOUNT_MENU_WIDTH: gpui::Pixels = px(240.);
const ACCOUNT_MENU_GAP: gpui::Pixels = px(6.);

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
struct CancelConversationRename;

pub struct SidebarView {
    collapsed: bool,
    conversations: Vec<ConversationSummary>,
    projects: Vec<magenta_core::Project>,
    expanded_projects: HashSet<PathBuf>,
    active_project: Option<PathBuf>,
    active_conversation: Option<ConversationId>,
    finder_launcher_focus: FocusHandle,
    account: Option<ProviderAccount>,
    pinned_expanded: bool,
    recency_limit: usize,
    history_status: Option<&'static str>,
    history_failed: bool,
    history_actions: HistoryActionState,
    rename: Option<RenameState>,
    rename_subscription: Option<Subscription>,
}

impl SidebarView {
    pub fn new(_window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        cx.bind_keys([KeyBinding::new(
            "escape",
            CancelConversationRename,
            Some("ConversationRename"),
        )]);
        Self {
            collapsed: false,
            conversations: Vec::new(),
            projects: Vec::new(),
            expanded_projects: HashSet::new(),
            active_project: None,
            active_conversation: None,
            finder_launcher_focus: cx.focus_handle(),
            account: None,
            pinned_expanded: true,
            recency_limit: INITIAL_RECENCY_LIMIT,
            history_status: Some("Loading conversations…"),
            history_failed: false,
            history_actions: HistoryActionState::Disabled,
            rename: None,
            rename_subscription: None,
        }
    }

    pub(crate) fn toggle_collapsed(&mut self, cx: &mut Context<'_, Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }

    pub(crate) const fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    #[cfg(test)]
    pub(crate) const fn active_conversation(&self) -> Option<ConversationId> {
        self.active_conversation
    }
    pub(crate) fn focus_finder_launcher(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.finder_launcher_focus.focus(window, cx);
    }

    pub(crate) fn matching_conversations(
        &self,
        query: &str,
    ) -> Vec<(ConversationId, SharedString, SharedString)> {
        self.conversations
            .iter()
            .filter(|conversation| title_matches(&conversation.title, query))
            .map(|conversation| {
                (
                    conversation.id,
                    conversation.title.clone().into(),
                    conversation.updated.clone().into(),
                )
            })
            .collect()
    }

    pub(crate) fn recent_conversations(&self) -> Vec<ConversationSummary> {
        self.conversations.iter().take(3).cloned().collect()
    }

    pub(crate) fn title_for(&self, id: ConversationId) -> Option<String> {
        self.conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.title.clone())
    }

    pub(crate) const fn history_available(&self) -> bool {
        self.history_status.is_none() && !self.conversations.is_empty()
    }

    fn new_chat(cx: &mut Context<'_, Self>) {
        cx.emit(SidebarEvent::NewChat);
        cx.notify();
    }

    fn select_conversation(id: ConversationId, cx: &mut Context<'_, Self>) {
        cx.emit(SidebarEvent::OpenConversation(id));
        cx.notify();
    }

    fn activate_project(&mut self, project: magenta_core::Project, cx: &mut Context<'_, Self>) {
        self.expanded_projects.insert(project.root.clone());
        self.active_project = Some(project.root.clone());
        cx.emit(SidebarEvent::ActivateProject(project));
        cx.notify();
    }

    fn project_conversations(&self, root: &Path) -> Vec<&ConversationSummary> {
        self.conversations
            .iter()
            .filter(|conversation| {
                conversation.mode == magenta_core::ConversationMode::Agent
                    && conversation.workspace_root.as_deref() == Some(root)
            })
            .collect()
    }

    fn belongs_to_registered_project(&self, conversation: &ConversationSummary) -> bool {
        conversation.mode == magenta_core::ConversationMode::Agent
            && conversation
                .workspace_root
                .as_ref()
                .is_some_and(|root| self.projects.iter().any(|project| project.root == *root))
    }

    pub(crate) fn set_projects(
        &mut self,
        projects: Vec<magenta_core::Project>,
        cx: &mut Context<'_, Self>,
    ) {
        self.projects = projects;
        cx.notify();
    }

    pub(crate) fn set_active_project(&mut self, root: Option<PathBuf>, cx: &mut Context<'_, Self>) {
        if let Some(root) = &root {
            self.expanded_projects.insert(root.clone());
        }
        self.active_project = root;
        cx.notify();
    }

    pub(crate) fn active_project(&self) -> Option<PathBuf> {
        self.active_project.clone()
    }

    pub(crate) fn active_project_details(&self) -> Option<magenta_core::Project> {
        let root = self.active_project.as_ref()?;
        self.projects
            .iter()
            .find(|project| &project.root == root)
            .cloned()
    }

    pub(crate) fn project_conversations_for(&self, root: &Path) -> Vec<ConversationSummary> {
        self.project_conversations(root)
            .into_iter()
            .cloned()
            .collect()
    }

    fn start_rename(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.rename.is_some() || !self.history_actions.is_enabled() {
            return;
        }
        let Some(original) = self
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.title.clone())
        else {
            return;
        };

        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |input, cx| {
            input.set_value(original.clone(), window, cx);
            input.set_selected_range(0..original.len(), cx);
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |sidebar, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => sidebar.clear_rename_validation(cx),
                InputEvent::PressEnter { shift: false, .. } | InputEvent::Blur => {
                    sidebar.commit_rename(window, cx);
                }
                _ => {}
            },
        );
        self.rename = Some(RenameState {
            id,
            original,
            input: input.clone(),
            invalid: false,
            saving: false,
        });
        self.rename_subscription = Some(subscription);
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn clear_rename_validation(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(rename) = &mut self.rename
            && rename.invalid
        {
            rename.invalid = false;
            cx.notify();
        }
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(rename) = &self.rename else {
            return;
        };
        if rename.saving {
            return;
        }

        let id = rename.id;
        let original = rename.original.clone();
        let input = rename.input.clone();
        let title = input.read(cx).value().trim().to_owned();
        if title.is_empty() {
            if let Some(rename) = &mut self.rename {
                rename.invalid = true;
            }
            input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
            return;
        }
        if title == original {
            self.cancel_rename(cx);
            return;
        }

        if let Some(rename) = &mut self.rename {
            rename.saving = true;
            rename.invalid = false;
        }
        input.update(cx, |input, cx| input.set_disabled(true, cx));
        cx.emit(SidebarEvent::RenameConversation(id, title));
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<'_, Self>) {
        self.rename = None;
        self.rename_subscription = None;
        cx.notify();
    }

    pub(crate) fn rename_succeeded(&mut self, id: ConversationId, cx: &mut Context<'_, Self>) {
        if self.rename.as_ref().is_some_and(|rename| rename.id == id) {
            self.cancel_rename(cx);
        }
    }

    pub(crate) fn rename_failed(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(rename) = self.rename.as_mut().filter(|rename| rename.id == id) else {
            return;
        };
        rename.saving = false;
        rename.invalid = true;
        let input = rename.input.clone();
        input.update(cx, |input, cx| input.set_disabled(false, cx));
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(crate) fn delete_succeeded(&mut self, id: ConversationId, cx: &mut Context<'_, Self>) {
        self.conversations
            .retain(|conversation| conversation.id != id);
        if self.rename.as_ref().is_some_and(|rename| rename.id == id) {
            self.cancel_rename(cx);
        }
        cx.notify();
    }

    pub(crate) fn set_history_actions_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let state = if enabled {
            HistoryActionState::Enabled
        } else {
            HistoryActionState::Disabled
        };
        if self.history_actions != state {
            self.history_actions = state;
            cx.notify();
        }
    }

    pub(crate) fn set_account(
        &mut self,
        account: Option<ProviderAccount>,
        cx: &mut Context<'_, Self>,
    ) {
        self.account = account;
        cx.notify();
    }

    fn set_pinned(&self, id: ConversationId, pinned: bool, cx: &mut Context<'_, Self>) {
        if !self.history_actions.is_enabled() {
            return;
        }
        if self
            .conversations
            .iter()
            .any(|conversation| conversation.id == id)
        {
            cx.emit(SidebarEvent::SetPinned(id, pinned));
        }
    }

    pub(crate) fn set_history(
        &mut self,
        summaries: Vec<magenta_core::ConversationSummary>,
        cx: &mut Context<'_, Self>,
    ) {
        self.conversations = summaries.into_iter().map(Into::into).collect();
        self.history_status = None;
        self.history_failed = false;
        cx.notify();
    }

    pub(crate) fn set_history_loading(&mut self, failed: bool, cx: &mut Context<'_, Self>) {
        self.history_failed = failed;
        self.history_status = Some(if failed {
            "History could not be loaded."
        } else {
            "Loading conversations…"
        });
        cx.notify();
    }

    pub(crate) fn set_active(&mut self, id: Option<ConversationId>, cx: &mut Context<'_, Self>) {
        self.active_conversation = id;
        cx.notify();
    }

    fn toggle_pinned_expanded(&mut self, cx: &mut Context<'_, Self>) {
        self.pinned_expanded = !self.pinned_expanded;
        cx.notify();
    }

    fn show_more(&mut self, cx: &mut Context<'_, Self>) {
        self.recency_limit = self.recency_limit.saturating_add(RECENCY_PAGE_SIZE);
        cx.notify();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileDetails {
    name: String,
    detail: String,
    initial: String,
    tooltip: String,
    email: Option<String>,
    plan: Option<String>,
}

fn profile_details(account: Option<&ProviderAccount>) -> ProfileDetails {
    let Some(account) = account else {
        return ProfileDetails {
            name: "Adleio".to_owned(),
            detail: "Local profile".to_owned(),
            initial: "A".to_owned(),
            tooltip: "Local profile and settings".to_owned(),
            email: None,
            plan: None,
        };
    };

    let name = account
        .name
        .clone()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| account.email.as_deref().and_then(email_display_name))
        .or_else(|| account.email.clone())
        .unwrap_or_else(|| "ChatGPT account".to_owned());
    let email = account.email.clone();
    let plan = account.plan.as_deref().and_then(display_plan);
    let detail = match (plan.as_deref(), email.as_deref()) {
        (Some(plan), Some(email)) => format!("{plan} · {email}"),
        (Some(plan), None) => plan.to_owned(),
        (None, Some(email)) => email.to_owned(),
        (None, None) => "OpenAI account".to_owned(),
    };
    let initial = name.chars().next().map_or_else(
        || "O".to_owned(),
        |character| character.to_uppercase().collect(),
    );
    let tooltip = account.email.clone().map_or_else(
        || "OpenAI account and settings".to_owned(),
        |email| format!("{name} · {email}"),
    );

    ProfileDetails {
        name,
        detail,
        initial,
        tooltip,
        email,
        plan,
    }
}

fn display_plan(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| capitalize(value))
}

fn email_display_name(email: &str) -> Option<String> {
    let local_part = email.split('@').next()?.trim();
    let parts = local_part
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(capitalize)
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };

    let mut capitalized = first.to_uppercase().collect::<String>();
    capitalized.push_str(characters.as_str());
    capitalized
}

impl EventEmitter<SidebarEvent> for SidebarView {}

#[derive(Clone)]
struct SidebarContent {
    view: Entity<SidebarView>,
    collapsed: bool,
}

impl Collapsible for SidebarContent {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for SidebarContent {
    fn render(
        self,
        id: impl Into<gpui::ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        v_flex()
            .id(id.into())
            .key_context("ConversationRename")
            .on_action(window.listener_for(
                &self.view,
                |sidebar, _: &CancelConversationRename, _, cx| sidebar.cancel_rename(cx),
            ))
            .w_full()
            .child(self.view.read(cx).render_content(self.view.clone(), cx))
    }
}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();
        Sidebar::new("magenta-sidebar")
            .w(EXPANDED_WIDTH)
            .collapsible(SidebarCollapsible::None)
            .child(SidebarContent {
                view: view.clone(),
                collapsed: false,
            })
            .footer(self.render_footer(&view, cx))
    }
}
