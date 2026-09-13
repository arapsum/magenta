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
