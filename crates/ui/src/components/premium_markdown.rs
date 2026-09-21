use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
    clipboard::Clipboard,
    h_flex,
    scroll::ScrollableElement as _,
    text::{
        MarkdownNode, MarkdownParseContext, MarkdownPlugin, TextView, TextViewStyle, markdown_ast,
    },
    v_flex,
};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rems,
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
        div()
            .w_full()
            .mt(px(12.))
            .mb(px(4.))
            .font_semibold()
            .text_size(px(19.))
            .line_height(px(26.))
            .text_color(cx.theme().foreground)
            .child(format!("{}. {}", heading.number, heading.title))
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

        let mut content = v_flex().w_full().gap(px(7.)).pb(rems(0.7));
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
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_none()
                            .w(px(22.))
                            .text_right()
                            .text_size(px(15.))
                            .line_height(px(25.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{}.", index + 1)),
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
        v_flex()
            .w_full()
            .mb(rems(0.75))
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.58))
            .bg(crate::components::visual::surface(
                crate::components::visual::SurfaceLevel::Raised,
                cx,
            ))
            .child(render_code_header(block, id, cx))
            .child(
                div().w_full().min_w_0().overflow_x_scrollbar().child(
                    TextView::markdown(("premium-code-body", id), fenced)
                        .selectable(true)
                        .style(code_style)
                        .w_full()
                        .text_size(px(13.))
                        .line_height(px(20.)),
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
        .border_color(cx.theme().border.opacity(0.42))
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
        paragraph_gap: rems(1.05),
        heading_base_font_size: px(16.),
        heading_font_size: Some(Arc::new(|level, _| match level {
            1 => px(30.),
            2 => px(23.),
            3 => px(19.),
            4 => px(17.),
            _ => px(16.),
        })),
        code_block: gpui_kit::StyleRefinement::default()
            .rounded(px(12.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.95))
            .bg(cx.theme().popover.opacity(0.88)),
        table: adaptive_table_style(),
        inline_code: gpui_kit::HighlightStyle {
            background_color: Some(cx.theme().accent.opacity(0.9)),
            ..Default::default()
        },
        is_dark: cx.theme().is_dark(),
        ..Default::default()
    }
}

fn adaptive_table_style() -> gpui_kit::StyleRefinement {
    let mut style = gpui_kit::StyleRefinement::default();
    style.overflow.x = Some(gpui_kit::Overflow::Scroll);
    style
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
        "rust" | "rs" => ("icons/language-rust.svg", true),
        "javascript" | "js" | "jsx" | "typescript" | "ts" | "tsx" => {
            ("icons/language-javascript.svg", true)
        }
        "kotlin" | "kt" | "kts" => ("icons/language-kotlin.svg", true),
        "c" => ("icons/language-c.svg", true),
        "c++" | "cpp" | "cxx" | "cc" | "hpp" => ("icons/language-cpp.svg", true),
        "zig" => ("icons/language-zig.svg", true),
        "json" | "jsonc" | "json5" => ("icons/language-json.svg", true),
        "python" | "py" | "py3" => ("icons/language-python.svg", true),
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
#[path = "../../test/components/premium_markdown.rs"]
mod tests;
