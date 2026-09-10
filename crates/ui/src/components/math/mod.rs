mod parsing;

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

use gpui_kit::component::{
    ActiveTheme as _, h_flex,
    text::{MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast},
};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, svg,
};
use parking_lot::Mutex;
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{SvgOptions, render_to_svg};
use ratex_types::{color::Color, math_style::MathStyle};

use crate::settings;

use self::parsing::{MathSegment, block_math_source, inline_math_segments};
use super::inline_code;

#[cfg(test)]
pub(super) use self::parsing::formulas;
pub(super) use self::parsing::{configured_formulas, normalize_delimiters};

const MATH_PLUGIN_NAME: &str = "magenta-math";
const DISPLAY_MATH_LANGUAGE: &str = "magenta-math";
const MAX_FORMULA_BYTES: usize = 16 * 1024;
const MAX_SVG_BYTES: usize = 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 128;
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
    font: magenta_core::MathFontStyle,
    font_size: u16,
}

impl FormulaKey {
    fn new(source: impl Into<String>, mode: MathMode) -> Self {
        let font_size = match mode {
            MathMode::Inline => 13,
            MathMode::Display => 16,
        };
        Self {
            source: source.into(),
            mode,
            font: magenta_core::MathFontStyle::Default,
            font_size,
        }
    }

    fn configured(source: impl Into<String>, mode: MathMode, cx: &App) -> Self {
        let font_size = match mode {
            MathMode::Inline => settings::inline_math_size(cx),
            MathMode::Display => settings::display_math_size(cx),
        };
        Self {
            source: source.into(),
            mode,
            font: settings::math_style(cx),
            font_size,
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

    pub(super) fn clear(&self) {
        self.formulas.lock().clear();
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

    let source = styled_source(&key.source, key.font);
    let ast = parse(&source).map_err(|error| MathError::Parse(error.to_string()))?;
    let font_size = f64::from(key.font_size);
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

fn styled_source(source: &str, font: magenta_core::MathFontStyle) -> String {
    let command = match font {
        magenta_core::MathFontStyle::Default => return source.to_owned(),
        magenta_core::MathFontStyle::Roman => "\\mathrm",
        magenta_core::MathFontStyle::SansSerif => "\\mathsf",
        magenta_core::MathFontStyle::Typewriter => "\\mathtt",
    };
    format!("{command}{{{source}}}")
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
    let key = FormulaKey::configured(source, mode, cx);
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
        assert!(rendered.width > gpui_kit::px(0.));
        assert!(rendered.height > gpui_kit::px(0.));
    }
}
