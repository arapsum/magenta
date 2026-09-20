use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, box_shadow,
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
    div, img, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, rems,
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
        let highlight = linear_gradient(
            90.,
            linear_color_stop(cx.theme().primary.opacity(0.), 0.),
            linear_color_stop(
                cx.theme()
                    .primary
                    .opacity(if focused { 0.92 } else { 0.48 }),
                1.,
            ),
        );

        v_flex()
            .w_full()
            .max_w(px(800.))
            .mx_auto()
            .items_center()
            .gap(px(14.))
            .child(self.mode_selector(cx.entity(), cx))
            .child(
                v_flex()
                    .id("prompt-composer-surface")
                    .debug_selector(|| "prompt-composer-surface".into())
                    .relative()
                    .w_full()
                    .p(px(2.))
                    .rounded(if is_work { px(22.) } else { px(19.) })
                    .border_1()
                    .border_color(if focused {
                        cx.theme().ring.opacity(0.78)
                    } else {
                        cx.theme().foreground.opacity(0.08)
                    })
                    .bg(super::super::visual::surface(
                        super::super::visual::SurfaceLevel::Floating,
                        cx,
                    ))
                    .shadow(super::super::visual::floating_shadow(cx))
                    .child(
                        div()
                            .absolute()
                            .top(px(0.))
                            .left(px(24.))
                            .right(px(24.))
                            .h(px(1.))
                            .bg(highlight),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .min_h(if is_work { px(124.) } else { px(56.) })
                            .p(if is_work { px(16.) } else { px(8.) })
                            .gap(if is_work { px(12.) } else { px(0.) })
                            .justify_between()
                            .rounded(if is_work { px(19.) } else { px(16.) })
                            .border_1()
                            .border_color(cx.theme().foreground.opacity(0.045))
                            .bg(super::super::visual::surface(
                                super::super::visual::SurfaceLevel::Raised,
                                cx,
                            ))
                            .shadow(super::super::visual::raised_shadow(cx))
                            .when(is_work, |this| {
                                this.child(self.render_input_content(cx)).child(self.footer(
                                    submit_view.clone(),
                                    generating,
                                    ready,
                                    false,
                                    cx,
                                ))
                            })
                            .when(!is_work, |this| {
                                this.child(self.render_chat_content(
                                    submit_view,
                                    generating,
                                    ready,
                                    can_add_attachment,
                                    cx,
                                ))
                            }),
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
