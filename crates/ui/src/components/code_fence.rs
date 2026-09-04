#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FencedCodeBlock {
    pub(super) source_start: usize,
    pub(super) marker_len: usize,
    pub(super) language: Option<String>,
    pub(super) code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ContentSegment {
    Text(String),
    Code(FencedCodeBlock),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AutoClose {
    pub(super) insertion: String,
}

#[derive(Clone, Copy)]
struct SourceLine<'a> {
    start: usize,
    next_start: usize,
    text: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FenceOpening {
    indent: usize,
    marker_len: usize,
    language: Option<String>,
}

pub(super) fn parse_segments(source: &str) -> Vec<ContentSegment> {
    let lines = source_lines(source);
    let mut segments = Vec::new();
    let mut plain_start = 0;
    let mut line_index = 0;

    while line_index < lines.len() {
        let Some(opening) = opening_fence(lines[line_index].text) else {
            line_index += 1;
            continue;
        };

        let closing_index = lines[line_index + 1..]
            .iter()
            .position(|line| is_closing_fence(line.text, opening.marker_len))
            .map(|offset| line_index + 1 + offset);
        let code_start = lines[line_index].next_start;
        let code_end = closing_index.map_or(source.len(), |index| lines[index].start);

        if plain_start < lines[line_index].start {
            segments.push(ContentSegment::Text(
                source[plain_start..lines[line_index].start].to_owned(),
            ));
        }

        segments.push(ContentSegment::Code(FencedCodeBlock {
            source_start: lines[line_index].start,
            marker_len: opening.marker_len,
            language: opening.language,
            code: source[code_start..code_end].to_owned(),
        }));

        if let Some(closing_index) = closing_index {
            plain_start = lines[closing_index].next_start;
            line_index = closing_index + 1;
        } else {
            plain_start = source.len();
            line_index = lines.len();
        }
    }

    if plain_start < source.len() {
        segments.push(ContentSegment::Text(source[plain_start..].to_owned()));
    }

    segments
}

pub(super) fn fenced_blocks(source: &str) -> Vec<FencedCodeBlock> {
    parse_segments(source)
        .into_iter()
        .filter_map(|segment| match segment {
            ContentSegment::Code(block) => Some(block),
            ContentSegment::Text(_) => None,
        })
        .collect()
}

pub(super) fn preview_markdown(blocks: &[FencedCodeBlock]) -> String {
    blocks
        .iter()
        .map(markdown_for_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn markdown_for_block(block: &FencedCodeBlock) -> String {
    let marker_len = block
        .marker_len
        .max(3)
        .max(longest_backtick_run(&block.code).saturating_add(1));
    let marker = "`".repeat(marker_len);
    let language = block.language.as_deref().unwrap_or_default().to_owned();
    let mut markdown = format!("{marker}{language}\n{}", block.code);

    if !block.code.ends_with('\n') {
        markdown.push('\n');
    }
    markdown.push_str(&marker);

    markdown
}

pub(super) fn opening_fence_after_newline(source: &str, cursor: usize) -> Option<AutoClose> {
    if cursor == 0 || cursor > source.len() || !source.is_char_boundary(cursor) {
        return None;
    }

    let prefix = &source[..cursor];
    if !prefix.ends_with('\n') {
        return None;
    }

    let line_end = cursor - 1;
    let line_start = source[..line_end]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let line = source[line_start..line_end]
        .strip_suffix('\r')
        .unwrap_or_else(|| &source[line_start..line_end]);
    let opening = opening_fence(line)?;

    if active_fence_marker(&source[..line_start]).is_some()
        || has_matching_closing_fence(&source[cursor..], opening.marker_len)
    {
        return None;
    }

    Some(AutoClose {
        insertion: format!(
            "\n{}{}",
            " ".repeat(opening.indent),
            "`".repeat(opening.marker_len),
        ),
    })
}

fn source_lines(source: &str) -> Vec<SourceLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;

    while start < source.len() {
        let relative_end = source[start..]
            .find('\n')
            .map_or(source.len() - start, |index| index + 1);
        let next_start = start + relative_end;
        let mut text_end = next_start;

        if source.as_bytes().get(text_end.saturating_sub(1)) == Some(&b'\n') {
            text_end -= 1;
            if source.as_bytes().get(text_end.saturating_sub(1)) == Some(&b'\r') {
                text_end -= 1;
            }
        }

        lines.push(SourceLine {
            start,
            next_start,
            text: &source[start..text_end],
        });
        start = next_start;
    }

    lines
}

fn opening_fence(line: &str) -> Option<FenceOpening> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }

    let marker_len = line[indent..]
        .bytes()
        .take_while(|byte| *byte == b'`')
        .count();
    if marker_len < 3 {
        return None;
    }

    let info = &line[indent + marker_len..];
    if info.contains('`') {
        return None;
    }

    Some(FenceOpening {
        indent,
        marker_len,
        language: info.split_whitespace().next().map(str::to_owned),
    })
}

fn is_closing_fence(line: &str, marker_len: usize) -> bool {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return false;
    }

    let run_len = line[indent..]
        .bytes()
        .take_while(|byte| *byte == b'`')
        .count();
    run_len >= marker_len && line[indent + run_len..].trim().is_empty()
}

fn active_fence_marker(source: &str) -> Option<usize> {
    let mut active_marker = None;

    for line in source_lines(source) {
        if let Some(marker_len) = active_marker {
            if is_closing_fence(line.text, marker_len) {
                active_marker = None;
            }
        } else if let Some(opening) = opening_fence(line.text) {
            active_marker = Some(opening.marker_len);
        }
    }

    active_marker
}

fn has_matching_closing_fence(source: &str, marker_len: usize) -> bool {
    source_lines(source)
        .iter()
        .any(|line| is_closing_fence(line.text, marker_len))
}

fn longest_backtick_run(source: &str) -> usize {
    source
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
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
}
