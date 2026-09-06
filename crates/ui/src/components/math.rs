use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, svg,
};
use gpui_component::{
    ActiveTheme as _, h_flex,
    text::{MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast},
};
use parking_lot::Mutex;
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{SvgOptions, render_to_svg};
use ratex_types::{color::Color, math_style::MathStyle};

use super::inline_code;

const MATH_PLUGIN_NAME: &str = "magenta-math";
const DISPLAY_MATH_LANGUAGE: &str = "magenta-math";
const MAX_FORMULA_BYTES: usize = 16 * 1024;
const MAX_SVG_BYTES: usize = 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 128;
const INLINE_FONT_SIZE: f64 = 13.;
const DISPLAY_FONT_SIZE: f64 = 16.;
const FORMULA_PADDING: f64 = 2.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MathMode {
    Inline,
    Display,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FormulaKey {
    source: String,
    mode: MathMode,
}

impl FormulaKey {
    fn new(source: impl Into<String>, mode: MathMode) -> Self {
        Self {
            source: source.into(),
            mode,
        }
    }
}

#[derive(Clone)]
pub(super) struct RenderedFormula {
    svg: Arc<str>,
    width: Pixels,
    height: Pixels,
}

#[derive(Clone)]
enum FormulaState {
    Pending,
    Ready(RenderedFormula),
    Failed,
}

#[derive(Default)]
pub(super) struct MathCache {
    formulas: Mutex<HashMap<FormulaKey, FormulaState>>,
}

impl MathCache {
    pub(super) fn begin(&self, key: FormulaKey) -> bool {
        let mut formulas = self.formulas.lock();
        if formulas.contains_key(&key) {
            return false;
        }

        if formulas.len() >= MAX_CACHE_ENTRIES {
            formulas.clear();
        }
        formulas.insert(key, FormulaState::Pending);
        true
    }

    pub(super) fn complete(&self, key: FormulaKey, result: Result<RenderedFormula, MathError>) {
        let state = result.map_or(FormulaState::Failed, FormulaState::Ready);
        self.formulas.lock().insert(key, state);
    }

    fn rendered(&self, key: &FormulaKey) -> Option<RenderedFormula> {
        match self.formulas.lock().get(key) {
            Some(FormulaState::Ready(formula)) => Some(formula.clone()),
            Some(FormulaState::Pending | FormulaState::Failed) | None => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum MathError {
    #[error("formula exceeds the {MAX_FORMULA_BYTES}-byte limit")]
    TooLarge,
    #[error("LaTeX could not be parsed: {0}")]
    Parse(String),
    #[error("rendered SVG exceeds the {MAX_SVG_BYTES}-byte limit")]
    SvgTooLarge,
}

pub(super) fn render_formula(key: &FormulaKey) -> Result<RenderedFormula, MathError> {
    if key.source.len() > MAX_FORMULA_BYTES {
        return Err(MathError::TooLarge);
    }

    let ast = parse(&key.source).map_err(|error| MathError::Parse(error.to_string()))?;
    let font_size = match key.mode {
        MathMode::Inline => INLINE_FONT_SIZE,
        MathMode::Display => DISPLAY_FONT_SIZE,
    };
    let style = match key.mode {
        MathMode::Inline => MathStyle::Text,
        MathMode::Display => MathStyle::Display,
    };
    let color = Color::rgb(0.9, 0.92, 0.94);
    let layout_box = layout(
        &ast,
        &LayoutOptions::default().with_style(style).with_color(color),
    );
    let display_list = to_display_list(&layout_box);
    let svg = render_to_svg(
        &display_list,
        &SvgOptions {
            font_size,
            padding: FORMULA_PADDING,
            stroke_width: 1.,
            embed_glyphs: true,
            font_dir: String::new(),
        },
    );
    if svg.len() > MAX_SVG_BYTES {
        return Err(MathError::SvgTooLarge);
    }

    let width = layout_box.width.mul_add(font_size, 2. * FORMULA_PADDING);
    let height = (layout_box.height + layout_box.depth).mul_add(font_size, 2. * FORMULA_PADDING);
    Ok(RenderedFormula {
        svg: Arc::from(svg),
        width: pixels_from_f64(width),
        height: pixels_from_f64(height),
    })
}

fn pixels_from_f64(value: f64) -> Pixels {
    let value = value.clamp(0., f64::from(f32::MAX));
    px(value.to_string().parse::<f32>().unwrap_or(f32::MAX))
}

#[derive(Clone)]
pub(super) struct MarkdownMathPlugin {
    cache: Arc<MathCache>,
}

impl MarkdownMathPlugin {
    pub(super) const fn new(cache: Arc<MathCache>) -> Self {
        Self { cache }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MathNode {
    Formula { source: String, mode: MathMode },
    Paragraph { segments: Vec<MathSegment> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MathSegment {
    source: String,
    math: bool,
}

impl MarkdownPlugin for MarkdownMathPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        MATH_PLUGIN_NAME
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        if let markdown_ast::Node::Code(code) = node
            && code.lang.as_deref() == Some(DISPLAY_MATH_LANGUAGE)
        {
            return Some(math_node(
                code.value.clone(),
                MathMode::Display,
                cx.node_source(node).unwrap_or_default(),
            ));
        }

        if let markdown_ast::Node::Math(math) = node {
            return Some(math_node(
                math.value.clone(),
                MathMode::Display,
                cx.node_source(node).unwrap_or_default(),
            ));
        }

        let markdown_ast::Node::Paragraph(_) = node else {
            return None;
        };
        let source = cx.node_source(node)?;

        if let Some(source) = block_math_source(source) {
            return Some(math_node(source.to_string(), MathMode::Display, source));
        }

        inline_math_segments(source).map(|segments| {
            MarkdownNode::new(MATH_PLUGIN_NAME, MathNode::Paragraph { segments })
                .text(source.to_string())
                .markdown(source.to_string())
        })
    }

    fn render(&self, node: &MarkdownNode, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(math) = node.data::<MathNode>() else {
            return div().child(node.as_text().to_owned()).into_any_element();
        };

        match math {
            MathNode::Formula { source, mode } => {
                let content = render_math_formula(source, *mode, &self.cache, cx);
                match mode {
                    MathMode::Inline => content,
                    MathMode::Display => div()
                        .id(("math-display", formula_id(source)))
                        .w_full()
                        .overflow_x_scroll()
                        .flex()
                        .justify_center()
                        .py_1()
                        .child(content)
                        .into_any_element(),
                }
            }
            MathNode::Paragraph { segments } => h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .children(segments.iter().map(|segment| {
                    if segment.math {
                        render_math_formula(&segment.source, MathMode::Inline, &self.cache, cx)
                    } else {
                        inline_code::render_plain_text(&segment.source, cx)
                    }
                }))
                .into_any_element(),
        }
    }
}

fn formula_id(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn render_math_formula(source: &str, mode: MathMode, cache: &MathCache, cx: &App) -> AnyElement {
    let key = FormulaKey::new(source, mode);
    if let Some(formula) = cache.rendered(&key) {
        return svg()
            .data(formula.svg.as_bytes())
            .w(formula.width)
            .h(formula.height)
            .flex_shrink_0()
            .text_color(cx.theme().foreground)
            .into_any_element();
    }

    let raw = match mode {
        MathMode::Inline => format!("${source}$"),
        MathMode::Display => format!("$$\n{source}\n$$"),
    };
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(cx.theme().muted_foreground)
        .child(raw)
        .into_any_element()
}

fn math_node(source: String, mode: MathMode, markdown: impl Into<String>) -> MarkdownNode {
    MarkdownNode::new(
        MATH_PLUGIN_NAME,
        MathNode::Formula {
            source: source.clone(),
            mode,
        },
    )
    .text(source)
    .markdown(markdown.into())
}

fn block_math_source(source: &str) -> Option<&str> {
    let source = source.trim();
    let body = source.strip_prefix("$$")?.strip_suffix("$$")?.trim();
    (!body.is_empty()).then_some(body)
}

fn inline_math_segments(source: &str) -> Option<Vec<MathSegment>> {
    let mut segments = Vec::new();
    let mut text_start = 0;
    let mut index = 0;
    let mut code_ticks = None;

    while index < source.len() {
        if let Some(ticks) = count_run(source, index, b'`') {
            if code_ticks == Some(ticks) {
                code_ticks = None;
            } else if code_ticks.is_none() {
                code_ticks = Some(ticks);
            }
            index += ticks;
            continue;
        }

        if code_ticks.is_none()
            && source.as_bytes()[index] == b'$'
            && source.as_bytes().get(index + 1) != Some(&b'$')
            && !is_escaped(source, index)
            && let Some(end) = find_inline_math_end(source, index + 1)
        {
            let math = source[index + 1..end].trim();
            if !math.is_empty() && !math.contains('\n') {
                if text_start < index {
                    segments.push(MathSegment {
                        source: source[text_start..index].to_string(),
                        math: false,
                    });
                }
                segments.push(MathSegment {
                    source: math.to_string(),
                    math: true,
                });
                index = end + 1;
                text_start = index;
                continue;
            }
        }

        index += source[index..].chars().next().map_or(1, char::len_utf8);
    }

    segments.iter().any(|segment| segment.math).then(|| {
        if text_start < source.len() {
            segments.push(MathSegment {
                source: source[text_start..].to_string(),
                math: false,
            });
        }
        segments
    })
}

pub(super) fn normalize_delimiters(source: &str) -> String {
    let canonical = canonicalize_delimiters(source);
    replace_display_math(&canonical)
}

fn canonicalize_delimiters(source: &str) -> String {
    let mut normalized = String::with_capacity(source.len());
    let mut code_fence = None;
    let mut display_open = false;
    let mut inline_open = false;

    for raw_line in source.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        if let Some(marker) = code_fence {
            normalized.push_str(raw_line);
            if is_fence_line(line, marker) {
                code_fence = None;
            }
            continue;
        }
        if let Some(marker) = opening_fence(line) {
            normalized.push_str(raw_line);
            code_fence = Some(marker);
            continue;
        }

        let mut index = 0;
        let mut code_ticks = None;
        while index < raw_line.len() {
            if let Some(ticks) = count_run(raw_line, index, b'`') {
                if code_ticks == Some(ticks) {
                    code_ticks = None;
                } else if code_ticks.is_none() {
                    code_ticks = Some(ticks);
                }
                normalized.push_str(&raw_line[index..index + ticks]);
                index += ticks;
                continue;
            }

            let rest = &raw_line[index..];
            if code_ticks.is_none() && !is_escaped(raw_line, index) {
                if display_open && rest.starts_with(r"\]") {
                    normalized.push_str("\n$$");
                    display_open = false;
                    index += 2;
                    continue;
                }
                if inline_open && rest.starts_with(r"\)") {
                    normalized.push('$');
                    inline_open = false;
                    index += 2;
                    continue;
                }
                if !display_open && !inline_open && rest.starts_with(r"\[") {
                    normalized.push_str("$$\n");
                    display_open = true;
                    index += 2;
                    continue;
                }
                if !display_open && !inline_open && rest.starts_with(r"\(") {
                    normalized.push('$');
                    inline_open = true;
                    index += 2;
                    continue;
                }
            }

            let width = raw_line[index..].chars().next().map_or(1, char::len_utf8);
            normalized.push_str(&raw_line[index..index + width]);
            index += width;
        }
    }

    normalized
}

pub(super) fn formulas(source: &str) -> Vec<FormulaKey> {
    let source = canonicalize_delimiters(source);
    let mut formulas = Vec::new();
    let mut index = 0;
    let mut code_ticks = None;
    let mut code_fence = None;

    while index < source.len() {
        let line_end = source[index..]
            .find('\n')
            .map_or(source.len(), |offset| index + offset);
        let line = &source[index..line_end];
        if let Some(marker) = code_fence {
            if is_fence_line(line, marker) {
                code_fence = None;
            }
            index = (line_end + 1).min(source.len());
            continue;
        }
        if let Some(marker) = opening_fence(line) {
            code_fence = Some(marker);
            index = (line_end + 1).min(source.len());
            continue;
        }

        if let Some(ticks) = count_run(&source, index, b'`') {
            if code_ticks == Some(ticks) {
                code_ticks = None;
            } else if code_ticks.is_none() {
                code_ticks = Some(ticks);
            }
            index += ticks;
            continue;
        }
        if code_ticks.is_some() {
            index += source[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        }

        if source.as_bytes()[index] == b'$' && !is_escaped(&source, index) {
            let display = source.as_bytes().get(index + 1) == Some(&b'$');
            let start = index + usize::from(display) + 1;
            if let Some(end) = find_math_end(&source, start, display) {
                let body = source[start..end].trim();
                if !body.is_empty() {
                    let key = FormulaKey::new(
                        body,
                        if display {
                            MathMode::Display
                        } else {
                            MathMode::Inline
                        },
                    );
                    if !formulas.contains(&key) {
                        formulas.push(key);
                    }
                }
                index = end + if display { 2 } else { 1 };
                continue;
            }
        }

        index += source[index..].chars().next().map_or(1, char::len_utf8);
    }

    formulas
}

fn replace_display_math(source: &str) -> String {
    let mut normalized = String::with_capacity(source.len());
    let mut code_fence = None;
    let mut index = 0;

    while index < source.len() {
        let line_end = source[index..]
            .find('\n')
            .map_or(source.len(), |offset| index + offset);
        let line = &source[index..line_end];
        if let Some(marker) = code_fence {
            normalized.push_str(&source[index..(line_end + 1).min(source.len())]);
            if is_fence_line(line, marker) {
                code_fence = None;
            }
            index = (line_end + 1).min(source.len());
            continue;
        }
        if let Some(marker) = opening_fence(line) {
            normalized.push_str(&source[index..(line_end + 1).min(source.len())]);
            code_fence = Some(marker);
            index = (line_end + 1).min(source.len());
            continue;
        }

        if source[index..].starts_with("$$")
            && !is_escaped(source, index)
            && let Some(end) = find_math_end(source, index + 2, true)
        {
            let body = source[index + 2..end].trim();
            if !body.is_empty() {
                normalized.push_str("\n```magenta-math\n");
                normalized.push_str(body);
                normalized.push_str("\n```\n");
                index = end + 2;
                continue;
            }
        }

        let width = source[index..].chars().next().map_or(1, char::len_utf8);
        normalized.push_str(&source[index..index + width]);
        index += width;
    }

    normalized
}

fn find_math_end(source: &str, mut index: usize, display: bool) -> Option<usize> {
    while index < source.len() {
        if source.as_bytes()[index] == b'$' && !is_escaped(source, index) {
            if display && source.as_bytes().get(index + 1) == Some(&b'$') {
                return Some(index);
            }
            if !display && source.as_bytes().get(index + 1) != Some(&b'$') {
                return Some(index);
            }
        }
        if !display && source.as_bytes()[index] == b'\n' {
            return None;
        }
        index += source[index..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

fn find_inline_math_end(source: &str, mut index: usize) -> Option<usize> {
    while index < source.len() {
        if source.as_bytes()[index] == b'$'
            && source.as_bytes().get(index + 1) != Some(&b'$')
            && !is_escaped(source, index)
        {
            return Some(index);
        }
        if source.as_bytes()[index] == b'\n' {
            return None;
        }
        index += source[index..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

fn count_run(source: &str, index: usize, needle: u8) -> Option<usize> {
    if source.as_bytes().get(index) != Some(&needle) {
        return None;
    }
    Some(
        source[index..]
            .bytes()
            .take_while(|byte| *byte == needle)
            .count(),
    )
}

fn is_escaped(source: &str, index: usize) -> bool {
    source[..index]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

fn opening_fence(line: &str) -> Option<u8> {
    let rest = line.trim_start_matches(' ');
    let marker = *rest.as_bytes().first()?;
    (marker == b'`' || marker == b'~')
        .then_some(marker)
        .filter(|marker| count_run(rest, 0, *marker).is_some_and(|length| length >= 3))
}

fn is_fence_line(line: &str, marker: u8) -> bool {
    let rest = line.trim_start_matches(' ');
    rest.as_bytes().first() == Some(&marker)
        && count_run(rest, 0, marker).is_some_and(|length| rest[length..].trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::{FormulaKey, MathMode, formulas, normalize_delimiters, render_formula};

    #[test]
    fn normalizes_backslash_delimiters_outside_code() {
        assert_eq!(
            normalize_delimiters(r"Use \(x^2\) and \[A = \pi r^2\]."),
            "Use $x^2$ and \n```magenta-math\nA = \\pi r^2\n```\n."
        );
    }

    #[test]
    fn leaves_backslash_delimiters_in_code_untouched() {
        let source = "`\\(x\\)`\n```text\n\\[x\\]\n```";
        assert_eq!(normalize_delimiters(source), source);
    }

    #[test]
    fn finds_inline_and_display_formulas() {
        assert_eq!(
            formulas("$x^2$\n\\[A = \\pi r^2\\]"),
            vec![
                FormulaKey::new("x^2", MathMode::Inline),
                FormulaKey::new("A = \\pi r^2", MathMode::Display),
            ]
        );
    }

    #[test]
    fn ignores_unclosed_formulas_and_code() {
        assert!(formulas("`$x$` and $unfinished").is_empty());
    }

    #[test]
    fn renders_a_display_formula_to_svg() {
        let rendered = render_formula(&FormulaKey::new("A = \\pi r^2", MathMode::Display))
            .expect("circle area formula should render");

        assert!(rendered.svg.starts_with("<svg"));
        assert!(rendered.width > gpui::px(0.));
        assert!(rendered.height > gpui::px(0.));
    }
}
