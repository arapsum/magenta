#[derive(Clone, Copy)]
struct Fence {
    marker: u8,
    length: usize,
}

/// Normalize Markdown constructs that the native text renderer does not support.
///
/// `gpui_component` currently treats hard line breaks inside a paragraph as an
/// unsupported inline node. A regular line break has the same conversational
/// appearance and avoids dropping content while the response is rendered.
/// Fenced code is copied byte-for-byte so formatting inside code blocks remains
/// intact.
pub(super) fn normalize_for_text_view(source: &str) -> String {
    let source = super::math::normalize_delimiters(source);
    let mut normalized = String::with_capacity(source.len());
    let mut active_fence = None;

    for raw_line in source.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(raw_line);

        if let Some(fence) = active_fence {
            normalized.push_str(raw_line);
            if is_closing_fence(line, fence) {
                active_fence = None;
            }
            continue;
        }

        if let Some(fence) = opening_fence(line) {
            normalized.push_str(raw_line);
            active_fence = Some(fence);
            continue;
        }

        normalized.push_str(normalize_plain_line(line, !line_ending.is_empty()));
        normalized.push_str(line_ending);
    }

    normalized
}

fn split_line_ending(raw_line: &str) -> (&str, &str) {
    let Some(without_lf) = raw_line.strip_suffix('\n') else {
        return (raw_line, "");
    };

    without_lf
        .strip_suffix('\r')
        .map_or((without_lf, "\n"), |line| (line, "\r\n"))
}

fn normalize_plain_line(line: &str, has_line_ending: bool) -> &str {
    let line = line.trim_end_matches([' ', '\t']);
    if !has_line_ending {
        return line;
    }

    let trailing_backslashes = line.bytes().rev().take_while(|byte| *byte == b'\\').count();
    if trailing_backslashes % 2 == 1 {
        &line[..line.len() - 1]
    } else {
        line
    }
}

fn opening_fence(line: &str) -> Option<Fence> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }

    let rest = line.get(indent..)?;
    let marker = *rest.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }

    let length = rest.bytes().take_while(|byte| *byte == marker).count();
    if length < 3 || (marker == b'`' && rest[length..].contains('`')) {
        return None;
    }

    Some(Fence { marker, length })
}

fn is_closing_fence(line: &str, fence: Fence) -> bool {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return false;
    }

    let Some(rest) = line.get(indent..) else {
        return false;
    };
    if rest.as_bytes().first().copied() != Some(fence.marker) {
        return false;
    }

    let length = rest
        .bytes()
        .take_while(|byte| *byte == fence.marker)
        .count();
    length >= fence.length && rest[length..].trim().is_empty()
}

#[cfg(test)]
mod tests {
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
}
