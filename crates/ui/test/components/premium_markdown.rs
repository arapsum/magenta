use super::{
    adaptive_table_style, display_language, fenced_markdown, language_icon, parse_step_heading,
};

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
    assert_eq!(language_icon("rust"), ("icons/language-rust.svg", true));
    assert_eq!(
        language_icon("TypeScript"),
        ("icons/language-javascript.svg", true)
    );
    assert_eq!(language_icon("kt"), ("icons/language-kotlin.svg", true));
    assert_eq!(language_icon("C"), ("icons/language-c.svg", true));
    assert_eq!(language_icon("cpp"), ("icons/language-cpp.svg", true));
    assert_eq!(language_icon("zig"), ("icons/language-zig.svg", true));
    assert_eq!(language_icon("jsonc"), ("icons/language-json.svg", true));
    assert_eq!(language_icon("py"), ("icons/language-python.svg", true));
    assert_eq!(language_icon("toml"), ("icons/code.svg", false));
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

#[test]
fn conversation_tables_use_adaptive_horizontal_layout() {
    assert_eq!(
        adaptive_table_style().overflow.x,
        Some(gpui_kit::Overflow::Scroll)
    );
}
