use super::normalize_for_text_view;

#[test]
fn converts_hard_breaks_to_regular_line_breaks() {
    assert_eq!(
        normalize_for_text_view("first  \nsecond\\\nthird"),
        "first\nsecond\nthird"
    );
}

#[test]
fn preserves_fenced_code_verbatim() {
    let source = "```rust\nlet value = 1;  \nprintln!(\\\"{value}\\\");\n```\n";

    assert_eq!(normalize_for_text_view(source), source);
}

#[test]
fn keeps_escaped_backslashes_and_crlf_line_endings() {
    let source = "path\\\\\r\nnext  \r\nfinal  ";

    assert_eq!(normalize_for_text_view(source), "path\\\\\r\nnext\r\nfinal");
}

#[test]
fn preserves_tilde_fenced_code() {
    let source = "~~~text\nline  \n~~~";

    assert_eq!(normalize_for_text_view(source), source);
}
