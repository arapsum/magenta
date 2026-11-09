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
fn zig_tokens_are_highlighted_in_code_and_markdown_with_both_themes() {
    use gpui_kit::component::{
        Rope,
        highlighter::{HighlightTheme, SyntaxHighlighter},
    };

    let code = "const std = @import(\"std\");\npub fn main() void {\n    std.debug.print(\"Hello, Zig!\\n\", .{});\n}\n";
    for (language, source) in [
        ("zig", code.to_owned()),
        ("markdown", fenced_markdown("zig", code)),
    ] {
        let mut highlighter = SyntaxHighlighter::new(language);
        assert!(highlighter.update(None, &Rope::from(source.as_str()), None));
        assert!(
            !highlighter
                .tree()
                .expect("the grammar must parse the code")
                .root_node()
                .has_error()
        );
        for theme in [
            HighlightTheme::default_dark(),
            HighlightTheme::default_light(),
        ] {
            let styles = highlighter.styles(&(0..source.len()), theme.as_ref());
            for token in ["const", "fn", "@import", "\"std\""] {
                assert!(
                    styles
                        .iter()
                        .any(|(range, style)| source.get(range.clone()) == Some(token)
                            && style.color.is_some()),
                    "{language} must assign a syntax color to {token}"
                );
            }
        }
    }
}

#[test]
fn conversation_tables_use_adaptive_horizontal_layout() {
    assert_eq!(
        adaptive_table_style().overflow.x,
        Some(gpui_kit::Overflow::Scroll)
    );
}
