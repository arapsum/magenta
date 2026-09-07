use std::io;

pub fn apply_unified(source: &str, patch: &str) -> io::Result<String> {
    let source_trailing_newline = source.ends_with('\n');
    let source_lines = source
        .trim_end_matches('\n')
        .split('\n')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let patch_lines = patch.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut source_index = 0_usize;
    let mut patch_index = 0_usize;
    let mut saw_hunk = false;

    while patch_index < patch_lines.len() {
        let line = patch_lines[patch_index];
        if !line.starts_with("@@") {
            patch_index += 1;
            continue;
        }
        saw_hunk = true;
        let old_start = hunk_start(line)?;
        let target_index = old_start.saturating_sub(1);
        if target_index < source_index || target_index > source_lines.len() {
            return Err(invalid("patch hunk starts outside the current file"));
        }
        output.extend(source_lines[source_index..target_index].iter().cloned());
        source_index = target_index;
        patch_index += 1;

        while patch_index < patch_lines.len() && !patch_lines[patch_index].starts_with("@@") {
            let change = patch_lines[patch_index];
            patch_index += 1;
            if change.starts_with("\\ No newline") || change.is_empty() {
                continue;
            }
            let (kind, value) = change.split_at(1);
            match kind {
                " " => {
                    expect_source(&source_lines, source_index, value)?;
                    output.push(value.to_owned());
                    source_index += 1;
                }
                "-" => {
                    expect_source(&source_lines, source_index, value)?;
                    source_index += 1;
                }
                "+" => output.push(value.to_owned()),
                _ => return Err(invalid("patch contains an unsupported hunk line")),
            }
        }
    }

    if !saw_hunk {
        return Err(invalid("patch did not contain a unified hunk"));
    }
    output.extend(source_lines[source_index..].iter().cloned());
    let mut result = output.join("\n");
    if source_trailing_newline {
        result.push('\n');
    }
    Ok(result)
}

fn hunk_start(line: &str) -> io::Result<usize> {
    let old = line
        .split_whitespace()
        .find(|part| part.starts_with('-'))
        .ok_or_else(|| invalid("patch hunk has no old range"))?;
    old.trim_start_matches('-')
        .split(',')
        .next()
        .ok_or_else(|| invalid("patch hunk has an invalid old range"))?
        .parse::<usize>()
        .map_err(|_| invalid("patch hunk has an invalid old range"))
}

fn expect_source(source: &[String], index: usize, expected: &str) -> io::Result<()> {
    if source.get(index).is_some_and(|line| line == expected) {
        Ok(())
    } else {
        Err(invalid("patch context does not match the current file"))
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::apply_unified;

    #[test]
    fn applies_a_small_unified_patch() {
        let source = "one\ntwo\nthree\n";
        let patch =
            "--- a/example.txt\n+++ b/example.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
        assert_eq!(apply_unified(source, patch).unwrap(), "one\nTWO\nthree\n");
    }

    #[test]
    fn rejects_stale_context() {
        let patch = "@@ -1,1 +1,1 @@\n-old\n+new\n";
        assert!(apply_unified("different\n", patch).is_err());
    }
}
