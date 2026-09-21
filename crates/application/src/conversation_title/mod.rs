const FALLBACK_MAX_WORDS: usize = 5;
const FALLBACK_MAX_CHARS: usize = 46;
const GENERATED_MAX_WORDS: usize = 5;
const GENERATED_MAX_CHARS: usize = 48;

pub fn fallback_from_prompt(prompt: &str, empty_title: &str) -> String {
    let title = prompt
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if title.is_empty() {
        return empty_title.to_owned();
    }

    compact_title(&title, FALLBACK_MAX_WORDS, FALLBACK_MAX_CHARS, true)
}

pub fn normalize_generated(value: &str) -> Option<String> {
    let mut title = value.split_whitespace().collect::<Vec<_>>().join(" ");

    if title.len() >= 2 {
        let quoted = (title.starts_with('"') && title.ends_with('"'))
            || (title.starts_with('`') && title.ends_with('`'));

        if quoted {
            title.remove(0);
            title.pop();
            trim_in_place(&mut title);
        }
    }

    if title
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("**title:**"))
    {
        title.drain(..10);
        trim_in_place(&mut title);
    }

    if title
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("title:"))
    {
        title.drain(..6);
        trim_in_place(&mut title);
    }

    while title.ends_with('.') {
        title.pop();
    }
    trim_in_place(&mut title);

    if title.is_empty() {
        return None;
    }

    Some(compact_title(
        &title,
        GENERATED_MAX_WORDS,
        GENERATED_MAX_CHARS,
        true,
    ))
}

fn compact_title(value: &str, max_words: usize, max_chars: usize, show_ellipsis: bool) -> String {
    let words = value.split_whitespace().collect::<Vec<_>>();
    let mut title = words
        .iter()
        .take(max_words)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let mut truncated = words.len() > max_words;

    let content_limit = max_chars.saturating_sub(usize::from(show_ellipsis && truncated));
    if title.chars().count() > content_limit {
        truncated = true;
        title = truncate_at_word_boundary(&title, max_chars.saturating_sub(1));
    }

    if show_ellipsis && truncated {
        title.push('…');
    }

    title
}

fn truncate_at_word_boundary(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }

    let candidate = value.chars().take(max_chars).collect::<String>();
    let boundary = candidate
        .char_indices()
        .rev()
        .find_map(|(index, character)| character.is_whitespace().then_some(index));

    boundary.map_or_else(|| candidate.clone(), |index| candidate[..index].to_owned())
}

fn trim_in_place(value: &mut String) {
    let leading = value.len().saturating_sub(value.trim_start().len());
    value.drain(..leading);
    value.truncate(value.trim_end().len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_titles_stop_at_word_boundaries() {
        assert_eq!(
            fallback_from_prompt(
                "Kotlin Projects: Organization, Dependencies, Builds, and Runtime Layout",
                "New conversation",
            ),
            "Kotlin Projects: Organization, Dependencies,…"
        );
    }

    #[test]
    fn generated_titles_are_normalized_and_bounded() {
        assert_eq!(
            normalize_generated("  **Title:**  Context Budget Design.  "),
            Some("Context Budget Design".to_owned())
        );
        assert_eq!(
            normalize_generated("\"Provider-Aware Conversation Titles.\""),
            Some("Provider-Aware Conversation Titles".to_owned())
        );
        assert!(normalize_generated("   ").is_none());

        let title = normalize_generated(&"x".repeat(80)).unwrap();
        assert!(title.chars().count() <= GENERATED_MAX_CHARS);
        assert!(title.ends_with('…'));
    }
}
