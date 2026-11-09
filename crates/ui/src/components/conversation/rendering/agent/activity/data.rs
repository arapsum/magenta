use super::*;

pub(super) fn entries(message: &Message, tab: TraceTab) -> Vec<&AssistantTraceEntry> {
    trace_entries_for_timeline(message)
        .into_iter()
        .filter(|entry| match tab {
            TraceTab::Steps => true,
            TraceTab::Reasoning => entry.kind == AssistantTraceKind::ReasoningSummary,
            TraceTab::Search => is_search(entry),
            TraceTab::Tools => entry.kind == AssistantTraceKind::Tool && !is_search(entry),
        })
        .collect()
}

pub(super) fn active_label(
    message: &Message,
    progress: Option<&GenerationProgress>,
) -> &'static str {
    if let Some(entry) = message
        .assistant_trace
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.status,
                AssistantTraceStatus::Running
                    | AssistantTraceStatus::Requested
                    | AssistantTraceStatus::AwaitingApproval
            )
        })
        .max_by_key(|entry| entry.sequence)
    {
        if entry.status == AssistantTraceStatus::AwaitingApproval {
            return "Waiting for approval";
        }
        return if is_search(entry) {
            "Searching"
        } else {
            "Working"
        };
    }
    if progress.is_some_and(|progress| {
        progress.message_id == message.id && progress.phase == GenerationPhase::Connecting
    }) {
        "Connecting"
    } else {
        "Thinking"
    }
}

fn is_search(entry: &AssistantTraceEntry) -> bool {
    entry.kind == AssistantTraceKind::Tool
        && matches!(
            entry.tool_name.as_deref(),
            Some("search_text" | "search_code" | "search_files" | "find_files")
        )
}

pub(super) fn argument(entry: &AssistantTraceEntry, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&entry.input)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

pub(super) fn target(entry: &AssistantTraceEntry) -> Option<String> {
    if matches!(
        entry.tool_name.as_deref(),
        Some("run_command" | "exec_command" | "shell")
    ) {
        let input: serde_json::Value = serde_json::from_str(&entry.input).ok()?;
        if let Some(command) = input.get("command").and_then(serde_json::Value::as_str) {
            return Some(command.to_owned());
        }
        let program = input.get("program")?.as_str()?;
        let args = input.get("args").and_then(serde_json::Value::as_array);
        return Some(
            std::iter::once(program)
                .chain(
                    args.into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str),
                )
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    argument(entry, "path").or_else(|| argument(entry, "query"))
}

pub(super) fn tool_verb(entry: &AssistantTraceEntry) -> String {
    match entry.tool_name.as_deref() {
        Some("read_file" | "read_skill" | "load_skill") => "Read".to_owned(),
        Some("list_files") => "List".to_owned(),
        Some("apply_patch" | "edit_file" | "write_file") => "Edit".to_owned(),
        Some("create_file" | "create_directory") => "Create".to_owned(),
        Some("run_command" | "exec_command" | "shell") => "Run".to_owned(),
        Some("view_image" | "read_image") => "View".to_owned(),
        _ => trace_entry_title(entry),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct DiffStats {
    pub added: usize,
    pub removed: usize,
}

pub(super) fn diff_stats(entry: &AssistantTraceEntry) -> Option<DiffStats> {
    if entry.status != AssistantTraceStatus::Completed {
        return None;
    }
    let diff = argument(entry, "unified_diff")?;
    let mut stats = DiffStats {
        added: 0,
        removed: 0,
    };
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("@@ ") {
            in_hunk = true;
        } else if line.starts_with("diff ") {
            in_hunk = false;
        } else if in_hunk {
            // Inside a hunk, even lines starting with +++ or --- are content.
            stats.added += usize::from(line.starts_with('+'));
            stats.removed += usize::from(line.starts_with('-'));
        }
    }
    Some(stats)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SearchMatch {
    pub path: String,
    pub location: String,
    pub excerpt: String,
}

pub(super) fn search_matches(entry: &AssistantTraceEntry) -> Vec<SearchMatch> {
    if entry.status != AssistantTraceStatus::Completed {
        return Vec::new();
    }
    match entry.tool_name.as_deref() {
        Some("search_text") => entry.output.lines().filter_map(text_match).collect(),
        Some("search_code") => entry
            .output
            .split("\n\n")
            .filter_map(|block| {
                let (header, content) = block.split_once('\n')?;
                let (path, location) = header.rsplit_once(':')?;
                let location = location.split_whitespace().next()?;
                let (start, end) = location.split_once('-')?;
                start.parse::<u32>().ok()?;
                end.parse::<u32>().ok()?;
                Some(SearchMatch {
                    path: path.to_owned(),
                    location: location.to_owned(),
                    excerpt: content.lines().next().unwrap_or_default().trim().to_owned(),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn text_match(line: &str) -> Option<SearchMatch> {
    // Locate the numeric line field instead of assuming paths contain no colons.
    for (index, _) in line.match_indices(':') {
        let tail = line.get(index + 1..)?;
        let (number, excerpt) = tail.split_once(':')?;
        if number.parse::<u32>().is_ok() {
            return Some(SearchMatch {
                path: line[..index].to_owned(),
                location: number.to_owned(),
                excerpt: excerpt.trim().to_owned(),
            });
        }
    }
    None
}
