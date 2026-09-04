use std::ops::Range;

use gpui::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, IntoElement, ParentElement as _,
    SharedString, Styled as _, StyledText, Window, div,
};
use gpui_component::ActiveTheme as _;
use gpui_component::text::{MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast};

const INLINE_CODE_PLUGIN_NAME: &str = "magenta-inline-code";

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MarkdownInlineCodePlugin;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InlineMark(u8);

impl InlineMark {
    const BOLD: Self = Self(1 << 0);
    const ITALIC: Self = Self(1 << 1);
    const STRIKETHROUGH: Self = Self(1 << 2);
    const CODE: Self = Self(1 << 3);

    const fn contains(self, mark: Self) -> bool {
        self.0 & mark.0 != 0
    }

    const fn with(self, mark: Self) -> Self {
        Self(self.0 | mark.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockKind {
    Paragraph,
    Heading(u8),
}

#[derive(Clone, Debug)]
struct InlineCodeBlock {
    kind: BlockKind,
    text: String,
    runs: Vec<InlineRun>,
}

#[derive(Clone, Debug)]
struct InlineRun {
    range: Range<usize>,
    mark: InlineMark,
}

impl MarkdownPlugin for MarkdownInlineCodePlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        INLINE_CODE_PLUGIN_NAME
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let (kind, children) = match node {
            markdown_ast::Node::Paragraph(paragraph) => {
                (BlockKind::Paragraph, paragraph.children.as_slice())
            }
            markdown_ast::Node::Heading(heading) => (
                BlockKind::Heading(heading.depth),
                heading.children.as_slice(),
            ),
            _ => return None,
        };

        let mut block = InlineCodeBlock {
            kind,
            text: String::new(),
            runs: Vec::new(),
        };
        if !append_nodes(children, InlineMark::default(), &mut block) {
            return None;
        }

        if !block
            .runs
            .iter()
            .any(|run| run.mark.contains(InlineMark::CODE))
        {
            return None;
        }

        let text = block.text.clone();
        Some(
            MarkdownNode::new(INLINE_CODE_PLUGIN_NAME, block)
                .text(text)
                .markdown(cx.node_source(node).unwrap_or_default()),
        )
    }

    fn render(&self, node: &MarkdownNode, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(block) = node.data::<InlineCodeBlock>() else {
            return div().child(node.as_text().to_owned()).into_any_element();
        };

        render_block(block, cx)
    }
}

pub(super) fn render_plain_text(text: &str, cx: &App) -> AnyElement {
    let Some((stripped, code_ranges)) = strip_code_spans(text) else {
        return div().child(text.to_owned()).into_any_element();
    };

    let runs: Vec<_> = code_ranges
        .into_iter()
        .map(|range| InlineRun {
            range,
            mark: InlineMark::CODE,
        })
        .collect();
    render_styled_text(stripped.to_string(), &runs, None, cx)
}

fn render_block(block: &InlineCodeBlock, cx: &App) -> AnyElement {
    let heading = match block.kind {
        BlockKind::Paragraph => None,
        BlockKind::Heading(level) => Some(level),
    };
    render_styled_text(block.text.clone(), &block.runs, heading, cx)
}

fn render_styled_text(
    text: String,
    runs: &[InlineRun],
    heading: Option<u8>,
    cx: &App,
) -> AnyElement {
    let mono_font = cx.theme().mono_font_family.clone();
    let highlights = runs
        .iter()
        .map(|run| (run.range.clone(), highlight_for(run.mark, cx)));
    let font_overrides = runs
        .iter()
        .filter(|run| run.mark.contains(InlineMark::CODE))
        .map(|run| (run.range.clone(), mono_font.clone()));
    let styled_text = StyledText::new(SharedString::from(text))
        .with_highlights(highlights)
        .with_font_family_overrides(font_overrides);

    let mut container = div().whitespace_normal();
    if let Some(level) = heading {
        let (size, weight) = match level {
            1 => (gpui::rems(2.), FontWeight::BOLD),
            2 => (gpui::rems(1.5), FontWeight::SEMIBOLD),
            3 => (gpui::rems(1.25), FontWeight::SEMIBOLD),
            4 => (gpui::rems(1.125), FontWeight::SEMIBOLD),
            5 => (gpui::rems(1.), FontWeight::SEMIBOLD),
            6 => (gpui::rems(1.), FontWeight::MEDIUM),
            _ => (gpui::rems(1.), FontWeight::NORMAL),
        };
        container = container.text_size(size).font_weight(weight);
    }

    container.child(styled_text).into_any_element()
}

fn highlight_for(mark: InlineMark, cx: &App) -> HighlightStyle {
    let mut highlight = HighlightStyle::default();
    if mark.contains(InlineMark::BOLD) {
        highlight.font_weight = Some(FontWeight::BOLD);
    }
    if mark.contains(InlineMark::ITALIC) {
        highlight.font_style = Some(FontStyle::Italic);
    }
    if mark.contains(InlineMark::STRIKETHROUGH) {
        highlight.strikethrough = Some(gpui::StrikethroughStyle {
            thickness: gpui::px(1.),
            ..Default::default()
        });
    }
    if mark.contains(InlineMark::CODE) {
        highlight.background_color = Some(cx.theme().accent);
    }
    highlight
}

fn append_nodes(
    nodes: &[markdown_ast::Node],
    mark: InlineMark,
    block: &mut InlineCodeBlock,
) -> bool {
    nodes.iter().all(|node| append_node(node, mark, block))
}

fn append_node(node: &markdown_ast::Node, mark: InlineMark, block: &mut InlineCodeBlock) -> bool {
    match node {
        markdown_ast::Node::Text(text) => append_run(&text.value, mark, block),
        markdown_ast::Node::InlineCode(code) => {
            append_run(&code.value, mark.with(InlineMark::CODE), block)
        }
        markdown_ast::Node::Emphasis(emphasis) => {
            append_nodes(&emphasis.children, mark.with(InlineMark::ITALIC), block)
        }
        markdown_ast::Node::Strong(strong) => {
            append_nodes(&strong.children, mark.with(InlineMark::BOLD), block)
        }
        markdown_ast::Node::Delete(delete) => append_nodes(
            &delete.children,
            mark.with(InlineMark::STRIKETHROUGH),
            block,
        ),
        markdown_ast::Node::Break(_) => append_run("\n", mark, block),
        _ => false,
    }
}

fn append_run(text: &str, mark: InlineMark, block: &mut InlineCodeBlock) -> bool {
    let start = block.text.len();
    block.text.push_str(text);
    let end = block.text.len();
    if start < end {
        block.runs.push(InlineRun {
            range: start..end,
            mark,
        });
    }
    true
}

fn strip_code_spans(text: &str) -> Option<(SharedString, Vec<Range<usize>>)> {
    if !text.contains('`') {
        return None;
    }

    let mut stripped = String::with_capacity(text.len());
    let mut code_ranges = Vec::new();
    let mut code_start = None;

    for character in text.chars() {
        if character == '`' {
            if let Some(start) = code_start.take() {
                code_ranges.push(start..stripped.len());
            } else {
                code_start = Some(stripped.len());
            }
        } else {
            stripped.push(character);
        }
    }

    if code_start.is_some() || code_ranges.is_empty() {
        return None;
    }

    Some((stripped.into(), code_ranges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_single_backtick_pairs_and_tracks_rendered_ranges() {
        let (text, ranges) = strip_code_spans("Use `ConversationId` here and `id`").unwrap();

        assert_eq!(text.as_ref(), "Use ConversationId here and id");
        assert_eq!(ranges, vec![4..18, 28..30]);
    }

    #[test]
    fn leaves_unmatched_backticks_literal() {
        assert!(strip_code_spans("Keep this `literal").is_none());
    }

    #[test]
    fn combines_nested_mark_and_code_ranges() {
        let mut block = InlineCodeBlock {
            kind: BlockKind::Paragraph,
            text: String::new(),
            runs: Vec::new(),
        };
        let nodes = vec![markdown_ast::Node::Strong(markdown_ast::Strong {
            children: vec![markdown_ast::Node::InlineCode(markdown_ast::InlineCode {
                value: "id".to_owned(),
                position: None,
            })],
            position: None,
        })];

        assert!(append_nodes(&nodes, InlineMark::default(), &mut block));
        assert_eq!(block.text, "id");
        assert_eq!(block.runs[0].range, 0..2);
        assert_eq!(block.runs[0].mark, InlineMark::BOLD.with(InlineMark::CODE));
    }
}
