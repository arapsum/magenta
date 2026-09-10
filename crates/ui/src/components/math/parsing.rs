use gpui_kit::App;

use super::{FormulaKey, MathMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MathSegment {
    pub(super) source: String,
    pub(super) math: bool,
}

pub(super) fn block_math_source(source: &str) -> Option<&str> {
    let source = source.trim();
    let body = source.strip_prefix("$$")?.strip_suffix("$$")?.trim();
    (!body.is_empty()).then_some(body)
}

pub(super) fn inline_math_segments(source: &str) -> Option<Vec<MathSegment>> {
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

pub(in crate::components) fn normalize_delimiters(source: &str) -> String {
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

#[cfg(test)]
pub(in crate::components) fn formulas(source: &str) -> Vec<FormulaKey> {
    formulas_with_settings(source, None)
}

pub(in crate::components) fn configured_formulas(source: &str, cx: &App) -> Vec<FormulaKey> {
    formulas_with_settings(source, Some(cx))
}

fn formulas_with_settings(source: &str, cx: Option<&App>) -> Vec<FormulaKey> {
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
                    let mode = if display {
                        MathMode::Display
                    } else {
                        MathMode::Inline
                    };
                    let key = cx.map_or_else(
                        || FormulaKey::new(body, mode),
                        |cx| FormulaKey::configured(body, mode, cx),
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
