# Alpha limitations and privacy

`0.1.0-alpha.1` is an unreleased, experimental candidate for technical Linux
users. This document defines the intended support boundary. Release packages
and the remaining qualification work are tracked in [the roadmap](../ROADMAP.md).

## Supported environment

The Alpha targets **Ubuntu 24.04 on Wayland and X11**, with working graphics
drivers and an unlocked Secret Service keyring. Other Linux distributions may
build from source, but are outside the Alpha support target. Windows and macOS
have no Alpha release qualification.

The reviewed source toolchain is Rust **1.99.1**, selected by
[`rust-toolchain.toml`](../rust-toolchain.toml). Native build dependencies are
listed in the [development guide](development.md).

OpenAI through ChatGPT browser sign-in is the sole production provider. Sign-in
uses a loopback callback on port 1455; account credentials are kept in the OS
keyring. Available models depend on the live catalog and account access.
Additional provider icons are visual assets.

Bubblewrap is optional for Chat and file-only Work. The desktop probes its
availability, but currently disables **all agent commands**, including when
Bubblewrap is installed, until commands can run against the reviewed AgentFS
overlay. Installing Bubblewrap alone does not enable commands in this candidate.

## Work and recovery boundaries

File tools stay beneath the selected workspace root and reject parent traversal,
escaping symlinks, and `.git` access. Protected reads require individual
approval; protected writes are rejected. Run-wide edit approval expires with
that run and does not authorize commands or protected reads.

Approved file and directory changes are staged in AgentFS. Final **Apply to
project** checks the original host content before writing; **Discard changes**
leaves the host files unchanged. The code workbench is read-only. Git staging,
unstaging, and confirmed commits are separate user actions in Changes.

Interrupted responses recover as stopped, but text streamed since turn creation
may be lost before terminal save. Failed saves retain visible output for retry.
Agent overlays and session records are retained locally; reopening an interrupted
review session after restart is not implemented. Back up important project files
and review staged changes before applying them.

History context uses approximate token counts and recent whole turns. It does
not summarize old conversations or automatically compact in-run tool continuations.
Workbench tabs and agent diff previews are not restored across sessions.

## Local data

On Linux, platform-local data usually means `~/.local/share` and configuration
usually means `~/.config`. `XDG_DATA_HOME` and `XDG_CONFIG_HOME` override them.

| Data | Location beneath the platform directory |
| --- | --- |
| Conversations, projects, assistant traces | Local data: `magenta/magenta.db` and database sidecars |
| Managed image copies | Local data: `magenta/attachments/` |
| Project memory, code index, content cache, session records | Local data: `magenta/projects/<root-hash>/agent.db` and sidecars |
| AgentFS overlays | Local data: `magenta/agentfs/` |
| Downloaded embedding models | Local data: `magenta/models/` |
| Daily rotating diagnostics, seven retained files | Local data: `magenta/logs/` |
| Appearance, generation, onboarding preferences | Configuration: `magenta/settings.toml` |
| Provider credentials | OS keyring service `dev.magenta.desktop`, account `openai-codex` |

History, attachments, project context, overlays, and diagnostics are local and
unencrypted. Traces and caches can contain tool inputs, outputs, and source code.
The desktop uses Turso; an old `conversations.sqlite3` is not automatically
imported or removed. Preserve it if upgrading from a legacy build.

Deleting a conversation removes its records and managed image copies, while
original image files and workspace files remain. Forgetting a project removes
its registration; it does not erase its project database, memory, overlays, or
workspace. There is no general local-data clearing or portable export workflow
yet. Do not interpret a sidebar action as erasure of all related local data.

## Network and diagnostics

Selected message context, attached images, retrieved project memory/code, and
agent tool results are sent to the connected provider to perform the request.
Before a Work run, supported source files can be indexed locally. The first
semantic operation downloads the embedding model to the local model cache;
later operations can reuse it offline. If embedding initialization fails,
retrieval falls back to lexical matching. Native inference runtime binaries
can also be downloaded while building dependencies.

There is no built-in telemetry or automatic diagnostic upload. Logs are local,
but technical error chains may include local details. Increasing `RUST_LOG`
verbosity can expose more diagnostic context. Review logs before sharing;
only structured response-failure details have an explicit allowlist.

## Backups and release gates

Close Magenta before backing up its complete local-data `magenta/` directory
and configuration directory, including database sidecars, attachments, project
databases, and overlays. Keyring credentials require the OS's own backup process.
Keep an untouched copy before an upgrade. A database changed by a newer build
may not be readable by an older build; downgrading the binary alone is not a
data rollback. Restore a matching closed-app backup when reverting versions.

Automated migration backups, conversation export, a full security review,
performance/accessibility evidence, and installation/rollback rehearsals remain
release work. A known data-loss, authentication, sandbox-escape, or
crash-on-launch defect blocks release, regardless of other completed milestones.
