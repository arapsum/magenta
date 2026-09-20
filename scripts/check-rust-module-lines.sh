#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(dirname -- "$script_directory")"
status=0

while IFS= read -r file; do
    lines="$(wc -l < "$file")"
    if (( lines > 800 )); then
        printf 'error: %s has %d lines (maximum 800)\n' "$file" "$lines"
        status=1
    elif (( lines > 750 )); then
        printf 'warning: %s has %d lines (target 750)\n' "$file" "$lines"
    fi
done < <(find "$repository_root/crates" -type f -name '*.rs' -print | sort)

while IFS= read -r module_file; do
    module_directory="${module_file%/mod.rs}"
    module_name="${module_directory##*/}"
    sibling="${module_directory%/*}/$module_name.rs"
    if [[ -f "$sibling" ]]; then
        printf 'error: directory module %s also has sibling registration %s\n' \
            "$module_file" "$sibling"
        status=1
    fi
done < <(find "$repository_root/crates" -type f -name mod.rs -print | sort)

exit "$status"
