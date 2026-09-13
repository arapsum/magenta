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
