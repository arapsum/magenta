use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, StyledExt as _,
    button::{Button, ButtonVariants},
    command::{Command, CommandItem},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use magenta_core::{CommandDescriptor, CommandPromptRequirement, ConversationMode, ProviderId};

use super::state::{CommandPaletteState, PromptComposer};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedSlashInput {
    token: String,
    subject: String,
}

fn parse_slash_input(value: &str) -> Option<ParsedSlashInput> {
    let value = value.strip_prefix('/')?;
    let token_end = value.find(char::is_whitespace).unwrap_or(value.len());
    let token = value[..token_end].to_owned();
    let subject = value[token_end..].trim_start().to_owned();

    Some(ParsedSlashInput { token, subject })
}

fn matching_command<'a>(
    commands: &'a [CommandDescriptor],
    token: &str,
) -> Option<&'a CommandDescriptor> {
    commands
        .iter()
        .find(|command| command.id.as_str().eq_ignore_ascii_case(token))
}

fn command_items(commands: &[CommandDescriptor], mode: &ConversationMode) -> Vec<CommandItem> {
    commands
        .iter()
        .map(|command| {
            let command_token = format!("/{}", command.id.as_str());
            let command_label = command.label.clone();
            let command_description = command.description.clone();
            let is_supported = command.supports_mode(mode);

            CommandItem::new()
                .label(format!("{command_token} {command_label}"))
                .keywords([
                    command.id.as_str().to_owned(),
                    command.label.clone(),
                    command.description.clone(),
                ])
                .child(move |_, cx| {
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_none()
                                .w(px(62.))
                                .font_medium()
                                .text_size(px(13.))
                                .text_color(cx.theme().primary)
                                .child(command_token.clone()),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .font_medium()
                                        .text_size(px(13.))
                                        .child(command_label.clone()),
                                )
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(px(11.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(command_description.clone()),
                                ),
                        )
                        .when(!is_supported, |this| {
                            this.child(
                                div()
                                    .flex_none()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .px(px(7.))
                                    .py(px(2.))
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Work only"),
                            )
                        })
                })
                .disabled(!is_supported)
        })
        .collect()
}

impl PromptComposer {
    fn command_provider(&self) -> ProviderId {
        self.model
            .as_ref()
            .map(|model| model.provider.clone())
            .or_else(|| {
                self.generation_settings
                    .for_mode(self.mode.clone())
                    .map(|preference| preference.provider.clone())
            })
            .or_else(|| self.models.first().map(|model| model.provider.clone()))
            .unwrap_or_else(|| ProviderId::new("openai"))
    }

    pub(crate) fn command_descriptors(&self) -> Vec<CommandDescriptor> {
        self.command_catalog
            .as_ref()
            .map_or_else(Vec::new, |catalog| {
                catalog.commands(&self.command_provider())
            })
    }

    pub(crate) fn sync_command_input(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.selected_command.is_some() {
            self.dismiss_command_palette(window, cx);
            return;
        }

        let value = self.input.read(cx).value().to_string();
        let Some(parsed) = parse_slash_input(&value) else {
            self.dismiss_command_palette(window, cx);
            return;
        };

        let commands = self.command_descriptors();
        if let Some(command) = matching_command(&commands, &parsed.token)
            && command.supports_mode(&self.mode)
        {
            self.select_command(command.clone(), parsed.subject, window, cx);
            return;
        }

        self.show_command_palette(&parsed.token, window, cx);
    }

    pub(crate) fn command_submission_ready(&self, cx: &App) -> bool {
        let Some(command_id) = self.selected_command.as_ref() else {
            return true;
        };
        let commands = self.command_descriptors();
        let Some(command) = matching_command(&commands, command_id.as_str()) else {
            return true;
        };

        match command.prompt_requirement {
            CommandPromptRequirement::Required => {
                !self.input.read(cx).value().trim().is_empty() || !self.attachments.is_empty()
            }
            CommandPromptRequirement::Optional { .. } => true,
        }
    }

    fn show_command_palette(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.command_palette_state = CommandPaletteState::Open;
        let query = query.to_owned();
        self.command_palette.update(cx, |state, cx| {
            state.set_query(query, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn dismiss_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.command_palette_state.is_open() {
            return;
        }

        self.command_palette_state = CommandPaletteState::Closed;
        self.command_palette.update(cx, |state, cx| {
            state.set_query("", window, cx);
        });
        self.focus(window, cx);
        cx.notify();
    }

    fn select_command(
        &mut self,
        command: CommandDescriptor,
        subject: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !command.supports_mode(&self.mode) {
            return;
        }

        self.selected_command = Some(command.id);
        self.input.update(cx, |input, cx| {
            input.set_value(subject, window, cx);
        });
        self.dismiss_command_palette(window, cx);
        cx.notify();
    }

    pub(crate) fn remove_selected_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected_command = None;
        self.dismiss_command_palette(window, cx);
        cx.notify();
    }

    pub(super) fn command_chip(&self, view: &Entity<Self>, cx: &Context<'_, Self>) -> AnyElement {
        let Some(command_id) = self.selected_command.as_ref() else {
            return div().into_any_element();
        };
        let label = format!("/{}", command_id.as_str());
        let remove_view = view.clone();

        h_flex()
            .id("prompt-command-chip")
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .h(px(26.))
            .rounded_full()
            .border_1()
            .border_color(cx.theme().primary.opacity(0.3))
            .bg(cx.theme().primary.opacity(0.08))
            .text_size(px(11.))
            .font_medium()
            .text_color(cx.theme().foreground)
            .child(label)
            .child(
                Button::new("remove-prompt-command")
                    .ghost()
                    .compact()
                    .size(px(18.))
                    .p_0()
                    .icon(IconName::CircleX)
                    .accessibility_label("Remove selected command")
                    .tooltip("Remove command")
                    .focus_visible(|this| this.text_color(cx.theme().ring))
                    .on_click(move |_, window, cx| {
                        remove_view.update(cx, |composer, cx| {
                            composer.remove_selected_command(window, cx);
                        });
                    }),
            )
            .into_any_element()
    }

    pub(super) fn command_palette_element(
        &self,
        view: &Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        if !self.command_palette_state.is_open() {
            return None;
        }

        let commands = self.command_descriptors();
        let mode = self.mode.clone();
        let confirm_view = view.clone();
        let cancel_view = view.clone();
        let query_view = view.clone();
        let tab_commands = commands.clone();
        let tab_state = self.command_palette.clone();
        let tab_view = view.clone();
        let items = command_items(&commands, &mode);

        let palette = Command::new(&self.command_palette)
            .items(items)
            .placeholder("Filter commands…")
            .max_h(px(204.))
            .bordered(false)
            .on_confirm(move |index, window, cx| {
                let Some(command) = commands.get(index.row).cloned() else {
                    return;
                };
                confirm_view.update(cx, |composer, cx| {
                    let input = composer.input.read(cx).value().to_string();
                    let subject =
                        parse_slash_input(&input).map_or_else(String::new, |parsed| parsed.subject);
                    composer.select_command(command, subject, window, cx);
                });
            })
            .on_query(move |query, window, cx| {
                if query.is_empty() {
                    query_view.update(cx, |composer, cx| {
                        composer.dismiss_command_palette(window, cx);
                    });
                }
            })
            .on_cancel(move |window, cx| {
                cancel_view.update(cx, |composer, cx| {
                    composer.dismiss_command_palette(window, cx);
                });
            });

        Some(
            div()
                .id("prompt-command-palette")
                .w_full()
                .aria_label("Slash command palette")
                .on_key_down(move |event, window, cx| {
                    if event.keystroke.key != "tab" || event.keystroke.modifiers.shift {
                        return;
                    }

                    let Some(index) = tab_state.read(cx).selected_index() else {
                        return;
                    };
                    let Some(command) = tab_commands.get(index.row).cloned() else {
                        return;
                    };
                    tab_view.update(cx, |composer, cx| {
                        let input = composer.input.read(cx).value().to_string();
                        let subject = parse_slash_input(&input)
                            .map_or_else(String::new, |parsed| parsed.subject);
                        composer.select_command(command, subject, window, cx);
                    });
                })
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(10.))
                .bg(cx.theme().popover)
                .shadow(super::super::visual::floating_shadow(cx))
                .child(palette)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::parse_slash_input;

    #[test]
    fn parses_command_and_subject() {
        assert_eq!(
            parse_slash_input("/plan refactor storage"),
            Some(super::ParsedSlashInput {
                token: "plan".to_owned(),
                subject: "refactor storage".to_owned(),
            })
        );
    }

    #[test]
    fn leaves_empty_subject_for_bare_command() {
        assert_eq!(
            parse_slash_input("/review"),
            Some(super::ParsedSlashInput {
                token: "review".to_owned(),
                subject: String::new(),
            })
        );
    }

    #[test]
    fn ignores_non_slash_input() {
        assert_eq!(parse_slash_input("remember this"), None);
    }
}
