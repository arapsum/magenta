use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, box_shadow,
    clipboard::Clipboard,
    h_flex,
    scroll::ScrollableElement as _,
    text::{
        MarkdownNode, MarkdownParseContext, MarkdownPlugin, TextView, TextViewStyle, markdown_ast,
    },
    v_flex,
};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Styled as _, Window, div, linear_color_stop,
    linear_gradient, prelude::FluentBuilder as _, px, rems,
};

use super::{
    inline_code::MarkdownInlineCodePlugin,
    math::{MarkdownMathPlugin, MathCache},
};

const ORDERED_LIST_PLUGIN: &str = "magenta-premium-ordered-list";
const CODE_BLOCK_PLUGIN: &str = "magenta-premium-code-block";
const STEP_HEADING_PLUGIN: &str = "magenta-premium-step-heading";

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PremiumStepHeadingPlugin;

#[derive(Clone, Debug)]
struct StepHeadingData {
    number: usize,
    title: String,
}

impl MarkdownPlugin for PremiumStepHeadingPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        STEP_HEADING_PLUGIN
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Heading(heading) = node else {
            return None;
        };
        if !(2..=4).contains(&heading.depth) {
            return None;
        }

        let source = cx.node_source(node)?;
        let heading = source
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_end_matches('#')
            .trim();
        let (number, title) = parse_step_heading(heading)?;

        Some(
            MarkdownNode::new(
                STEP_HEADING_PLUGIN,
                StepHeadingData {
                    number,
                    title: title.to_owned(),
                },
            )
            .text(heading)
            .markdown(source),
        )
    }

    fn render(&self, node: &MarkdownNode, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(heading) = node.data::<StepHeadingData>() else {
            return div().child(node.as_text().to_owned()).into_any_element();
        };
        let marker = linear_gradient(
            145.,
            linear_color_stop(cx.theme().primary.opacity(0.84), 0.),
            linear_color_stop(cx.theme().accent.opacity(0.96), 1.),
        );

        h_flex()
            .w_full()
            .ml(px(-46.))
            .mt(px(8.))
            .mb(px(3.))
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .flex_none()
                    .size(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().primary.opacity(0.34))
                    .bg(marker)
                    .text_color(cx.theme().primary_foreground)
                    .font_semibold()
                    .text_size(px(13.))
                    .shadow(vec![box_shadow(
                        0.,
                        5.,
                        14.,
                        -5.,
                        cx.theme().primary.opacity(0.42),
                    )])
                    .child(heading.number.to_string()),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .font_semibold()
                    .text_size(px(18.))
                    .line_height(px(24.))
                    .text_color(cx.theme().foreground)
                    .child(heading.title.clone()),
            )
            .into_any_element()
    }
}

#[derive(Clone)]
pub(super) struct PremiumOrderedListPlugin {
    math_cache: Arc<MathCache>,
}

impl PremiumOrderedListPlugin {
    pub(super) const fn new(math_cache: Arc<MathCache>) -> Self {
        Self { math_cache }
    }
}

#[derive(Clone, Debug)]
struct OrderedListData {
    items: Vec<String>,
    source: String,
}

impl MarkdownPlugin for PremiumOrderedListPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        ORDERED_LIST_PLUGIN
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::List(list) = node else {
            return None;
        };
        if !list.ordered {
            return None;
        }

        let items = list
            .children
            .iter()
            .filter_map(|node| {
                let markdown_ast::Node::ListItem(item) = node else {
                    return None;
                };
                let body = item
                    .children
                    .iter()
                    .filter_map(|child| cx.node_source(child))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                (!body.trim().is_empty()).then_some(body)
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return None;
        }

        let source = cx.node_source(node).unwrap_or_default().to_owned();
        Some(
            MarkdownNode::new(
                ORDERED_LIST_PLUGIN,
                OrderedListData {
                    items,
                    source: source.clone(),
                },
            )
            .text(source.clone())
            .markdown(source),
        )
    }

    fn render(&self, node: &MarkdownNode, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(list) = node.data::<OrderedListData>() else {
            return div().child(node.as_text().to_owned()).into_any_element();
        };

        let mut content = v_flex().w_full().gap(px(12.)).pb(rems(0.85));
        for (index, item) in list.items.iter().enumerate() {
            let id = content_id(&list.source, index);
            let text = TextView::markdown(("premium-list-item", id), item.clone())
                .selectable(true)
                .plugin(MarkdownMathPlugin::new(self.math_cache.clone()))
                .plugin(MarkdownInlineCodePlugin)
                .style(conversation_text_style(cx))
                .w_full()
                .text_size(px(16.))
                .line_height(px(25.));
            content = content.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .mt(px(1.))
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(cx.theme().primary.opacity(0.2))
                            .text_color(cx.theme().primary)
                            .font_medium()
                            .text_size(px(13.))
                            .child((index + 1).to_string()),
                    )
                    .child(div().min_w_0().flex_1().child(text)),
            );
        }
        content.into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PremiumCodeBlockPlugin;

#[derive(Clone, Debug)]
struct CodeBlockData {
    language: String,
    code: String,
    source: String,
}

impl MarkdownPlugin for PremiumCodeBlockPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        CODE_BLOCK_PLUGIN
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Code(code) = node else {
            return None;
        };
        let source = cx.node_source(node).unwrap_or_default().to_owned();
        let language = code.lang.clone().unwrap_or_else(|| "text".to_owned());
        Some(
            MarkdownNode::new(
                CODE_BLOCK_PLUGIN,
                CodeBlockData {
                    language,
                    code: code.value.clone(),
                    source: source.clone(),
                },
            )
            .text(code.value.clone())
            .markdown(source),
        )
    }

    fn render(&self, node: &MarkdownNode, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(block) = node.data::<CodeBlockData>() else {
            return div().child(node.as_text().to_owned()).into_any_element();
        };
        let id = content_id(&block.source, 0);
        let fenced = fenced_markdown(&block.language, &block.code);
        let surface = linear_gradient(
            145.,
            linear_color_stop(cx.theme().popover.opacity(0.98), 0.),
            linear_color_stop(cx.theme().accent.opacity(0.34), 1.),
        );
        let code_style = TextViewStyle {
            paragraph_gap: rems(0.),
            code_block: gpui_kit::StyleRefinement::default()
                .p(px(12.))
                .rounded(px(0.))
                .border_0()
                .bg(cx.theme().transparent),
            is_dark: cx.theme().is_dark(),
            ..Default::default()
        };
        let line_numbers = (1..=block.code.lines().count().max(1))
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        v_flex()
            .w_full()
            .mb(rems(0.8))
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().primary.opacity(0.2))
            .bg(surface)
            .shadow(vec![
                box_shadow(0., 16., 34., -20., cx.theme().primary.opacity(0.34)),
                box_shadow(0., 5., 14., -8., cx.theme().background.opacity(0.86)),
            ])
            .child(render_code_header(block, id, cx))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .child(
                        div()
                            .w(px(42.))
                            .flex_none()
                            .py(px(12.))
                            .pr(px(10.))
                            .border_r_1()
                            .border_color(cx.theme().border.opacity(0.62))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(11.))
                            .line_height(px(20.))
                            .text_right()
                            .text_color(cx.theme().muted_foreground.opacity(0.62))
                            .child(line_numbers),
                    )
                    .child(
                        div().min_w_0().flex_1().overflow_x_scrollbar().child(
                            TextView::markdown(("premium-code-body", id), fenced)
                                .selectable(true)
                                .style(code_style)
                                .w_full()
                                .text_size(px(13.))
                                .line_height(px(20.)),
                        ),
                    ),
            )
            .into_any_element()
    }
}

fn render_code_header(block: &CodeBlockData, id: u64, cx: &App) -> AnyElement {
    let (language_icon, is_branded_icon) = language_icon(&block.language);
    h_flex()
        .h(px(38.))
        .flex_none()
        .items_center()
        .justify_between()
        .px(px(12.))
        .border_b_1()
        .border_color(cx.theme().primary.opacity(0.16))
        .bg(cx.theme().accent.opacity(0.42))
        .child(
            h_flex()
                .items_center()
                .gap(px(7.))
                .text_color(cx.theme().muted_foreground)
                .child(
                    Icon::empty()
                        .path(language_icon)
                        .small()
                        .when(!is_branded_icon, |icon| icon.text_color(cx.theme().primary)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .font_medium()
                        .child(display_language(&block.language)),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(2.))
                .text_size(px(11.))
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(
                    Clipboard::new(("copy-premium-code", id))
                        .value(block.code.clone())
                        .tooltip("Copy code"),
                )
                .child("Copy"),
        )
        .into_any_element()
}

pub(super) fn conversation_text_style(cx: &App) -> TextViewStyle {
    TextViewStyle {
        paragraph_gap: rems(0.9),
        heading_base_font_size: px(16.),
        heading_font_size: Some(Arc::new(|level, _| match level {
            1 => px(30.),
            2 => px(24.),
            3 => px(19.),
            4 => px(17.),
            _ => px(16.),
        })),
        code_block: gpui_kit::StyleRefinement::default()
            .rounded(px(12.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.95))
            .bg(cx.theme().popover.opacity(0.88)),
        inline_code: gpui_kit::HighlightStyle {
            background_color: Some(cx.theme().accent.opacity(0.9)),
            ..Default::default()
        },
        is_dark: cx.theme().is_dark(),
        ..Default::default()
    }
}

fn display_language(language: &str) -> String {
    let language = language.trim();
    if language.is_empty() || language.eq_ignore_ascii_case("text") {
        "Code".to_owned()
    } else {
        let mut chars = language.chars();
        chars.next().map_or_else(
            || "Code".to_owned(),
            |first| first.to_uppercase().chain(chars).collect(),
        )
    }
}

fn language_icon(language: &str) -> (&'static str, bool) {
    match language.trim().to_ascii_lowercase().as_str() {
        "java" => ("icons/language-java.svg", true),
        "c#" | "cs" | "csharp" | "dotnet" => ("icons/language-csharp.svg", true),
        "bash" | "sh" | "shell" | "zsh" => ("icons/language-bash.svg", true),
        _ => ("icons/code.svg", false),
    }
}

fn parse_step_heading(heading: &str) -> Option<(usize, &str)> {
    let (number, title) = heading.split_once('.')?;
    let number = number.trim().parse().ok()?;
    let title = title.trim();
    (!title.is_empty()).then_some((number, title))
}

fn content_id(source: &str, index: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    index.hash(&mut hasher);
    hasher.finish()
}

fn fenced_markdown(language: &str, code: &str) -> String {
    let longest_run = code
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let marker = "`".repeat(longest_run.saturating_add(1).max(3));
    format!("{marker}{language}\n{code}\n{marker}")
}

#[cfg(test)]
mod tests {
    use super::{display_language, fenced_markdown, language_icon, parse_step_heading};

    #[test]
    fn display_language_is_human_readable() {
        assert_eq!(display_language("rust"), "Rust");
        assert_eq!(display_language("text"), "Code");
    }

    #[test]
    fn language_icons_cover_supplied_aliases() {
        assert_eq!(language_icon("java"), ("icons/language-java.svg", true));
        assert_eq!(language_icon("C#"), ("icons/language-csharp.svg", true));
        assert_eq!(language_icon("zsh"), ("icons/language-bash.svg", true));
        assert_eq!(language_icon("rust"), ("icons/code.svg", false));
    }

    #[test]
    fn numbered_heading_is_recognized_as_a_step() {
        assert_eq!(
            parse_step_heading("2. Compilation to bytecode"),
            Some((2, "Compilation to bytecode"))
        );
        assert_eq!(parse_step_heading("Summary"), None);
    }

    #[test]
    fn fenced_markdown_uses_a_marker_longer_than_the_code() {
        let markdown = fenced_markdown("rust", "let ticks = ```;");
        assert!(markdown.starts_with("````rust\n"));
        assert!(markdown.ends_with("\n````"));
    }
}
