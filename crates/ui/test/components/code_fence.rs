use super::*;

#[test]
fn parses_prose_and_fenced_code() {
    let source = "Before\n```rust\nfn main() {}\n```\nAfter";

    assert_eq!(
        parse_segments(source),
        vec![
            ContentSegment::Text("Before\n".to_owned()),
            ContentSegment::Code(FencedCodeBlock {
                source_start: 7,
                marker_len: 3,
                language: Some("rust".to_owned()),
                code: "fn main() {}\n".to_owned(),
            }),
            ContentSegment::Text("After".to_owned()),
        ]
    );
}

#[test]
fn treats_unclosed_backtick_fence_as_code_to_eof() {
    let segments = parse_segments("```rust\nlet answer = 42;");

    assert_eq!(
        segments,
        vec![ContentSegment::Code(FencedCodeBlock {
            source_start: 0,
            marker_len: 3,
            language: Some("rust".to_owned()),
            code: "let answer = 42;".to_owned(),
        })]
    );
}

#[test]
fn preserves_tilde_fences_as_literal_text() {
    assert_eq!(
        parse_segments("~~~rust\nlet answer = 42;\n~~~"),
        vec![ContentSegment::Text(
            "~~~rust\nlet answer = 42;\n~~~".to_owned()
        )]
    );
}

#[test]
fn preview_uses_a_safe_marker_for_code_containing_backticks() {
    let blocks = fenced_blocks("```text\nvalue = ```\n");
    let preview = preview_markdown(&blocks);

    assert!(preview.starts_with("````text\n"));
    assert!(preview.ends_with("\n````"));
    assert_eq!(fenced_blocks(&preview)[0].code, "value = ```\n");
}

#[test]
fn pairs_a_newly_opened_fence_with_matching_indentation() {
    let source = "  ```rust\n";

    assert_eq!(
        opening_fence_after_newline(source, source.len()),
        Some(AutoClose {
            insertion: "\n  ```".to_owned()
        })
    );
}

#[test]
fn does_not_pair_a_closing_fence_or_existing_pair() {
    let closed = "```\ncode\n```\n";
    let already_paired = "```\n\n```\n";

    assert_eq!(opening_fence_after_newline(closed, closed.len()), None);
    assert_eq!(
        opening_fence_after_newline(already_paired, "```\n".len()),
        None
    );
}

#[test]
fn does_not_pair_inline_or_nested_fences() {
    let inline = "Ask for ```rust\n";
    let nested = "```\n```\n";

    assert_eq!(opening_fence_after_newline(inline, inline.len()), None);
    assert_eq!(opening_fence_after_newline(nested, nested.len()), None);
}
