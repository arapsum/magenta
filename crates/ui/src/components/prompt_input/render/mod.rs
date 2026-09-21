use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants, Toggle, ToggleGroup},
    clipboard::Clipboard,
    h_flex,
    input::Textarea,
    popover::{Popover, PopoverState},
    text::{TextView, TextViewStyle},
    v_flex,
};
use gpui_kit::{
    Anchor, AnyElement, App, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Render, SharedString, Styled as _, StyledImage as _, Window,
    div, img, prelude::FluentBuilder as _, px, rems,
};
use magenta_core::{ConversationMode, EffortLevel, ModelDescriptor, ProviderId};

use crate::components::provider_icon;

use super::{MAX_ATTACHMENTS, PromptComposer, PromptWorkspacePanel};

mod composer;
mod picker;
mod workspace;

impl Render for PromptComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        self.synchronize_input_mode(window, cx);

        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.is_ready(cx) && !self.generating;
        let submit_view = cx.entity();
        let generating = self.generating;
        let is_work = self.mode == ConversationMode::Agent;
        let can_add_attachment = !is_work && self.attachments.len() < MAX_ATTACHMENTS;
        v_flex()
            .w_full()
            .max_w(px(880.))
            .mx_auto()
            .items_center()
            .child(
                v_flex()
                    .id("prompt-composer-surface")
                    .debug_selector(|| "prompt-composer-surface".into())
                    .relative()
                    .w_full()
                    .p(px(14.))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(if focused && !generating {
                        cx.theme().ring.opacity(0.72)
                    } else {
                        cx.theme().border.opacity(0.82)
                    })
                    .bg(super::super::visual::surface(
                        super::super::visual::SurfaceLevel::Floating,
                        cx,
                    ))
                    .shadow(super::super::visual::floating_shadow(cx))
                    .child(
                        v_flex()
                            .w_full()
                            .min_h(px(88.))
                            .gap(px(12.))
                            .justify_between()
                            .child(self.render_input_content(cx))
                            .child(self.footer(
                                submit_view,
                                generating,
                                ready,
                                can_add_attachment,
                                cx,
                            )),
                    ),
            )
    }
}

fn provider_menu_label(provider: &ProviderId) -> String {
    let id = provider.0.to_ascii_lowercase();
    if id.starts_with("openai") {
        "OpenAI".to_owned()
    } else if id.starts_with("anthropic") || id.starts_with("claude") {
        "Anthropic".to_owned()
    } else if id.starts_with("google") || id.starts_with("gemini") {
        "Gemini".to_owned()
    } else {
        provider.0.clone()
    }
}

fn picker_heading(label: &'static str, cx: &App) -> AnyElement {
    div()
        .h(px(28.))
        .px(px(9.))
        .flex()
        .items_center()
        .text_size(px(10.))
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

const fn effort_description(effort: &EffortLevel) -> &'static str {
    match effort {
        EffortLevel::None | EffortLevel::Minimal => "Fastest",
        EffortLevel::Low => "Quick",
        EffortLevel::Medium => "Balanced",
        EffortLevel::High => "Deeper",
        EffortLevel::XHigh | EffortLevel::Max => "Deepest",
        EffortLevel::Custom { .. } => "Custom",
    }
}
