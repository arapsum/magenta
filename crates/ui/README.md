# magenta-ui

Magenta's native GPUI application shell and components. This crate depends on
`magenta-application` and `magenta-core`; concrete storage, provider, and
workspace adapters are injected by the desktop crate. UI code owns interaction
state, rendering, background-task lifetimes, and recovery presentation.
[Workspace overview](../../README.md).

## Public entry points

[src/lib.rs](src/lib.rs) exports `MainView`, `MainServices`, `init_settings`,
`MagentaError`, `Result`, and error-presentation helpers. `MainServices` supplies
account/model/settings ports plus optional agent and project workflows.

The public [theme module](src/theme.rs) registers bundled themes and provides
`init`, `available`, `apply`, `apply_named`, and `toggle`. `ThemeInitOutcome`
allows desktop startup to report a theme failure while using a default theme.
Initialize GPUI Kit and settings globals before constructing the main view;
the [desktop entry point](../desktop/src/main.rs) shows the complete setup.

## Source map

| Area | Source and responsibility |
| --- | --- |
| Main application | [app/mod.rs](src/app/mod.rs), [app/render](src/app/render/mod.rs): services, view composition, layout |
| History and recovery | [app/history](src/app/history/mod.rs): loading, turn operations, terminal saves, deferred navigation |
| Accounts and settings | [app/account](src/app/account/mod.rs), [app/settings_window](src/app/settings_window/mod.rs): login, model discovery, separate Settings window |
| Search and projects | [app/finder](src/app/finder/mod.rs), [app/projects](src/app/projects/mod.rs): search/navigation and workspace selection |
| Sidebar and landing | [components/sidebar](src/components/sidebar/mod.rs), [workspace.rs](src/components/workspace.rs): navigation, projects, account controls, start screen |
| Composer | [components/prompt_input](src/components/prompt_input/mod.rs): input, attachments, modes, model/effort controls, inline errors |
| Conversation | [components/conversation](src/components/conversation/mod.rs): paged rendering, streaming, approvals, activity and failures |
| Workbench | [components/agent_workbench](src/components/agent_workbench/mod.rs): project tree, file tabs, read-only code/diff review |
| Rich content | [markdown.rs](src/components/markdown.rs), [inline_code.rs](src/components/inline_code.rs), [math](src/components/math/mod.rs): text and RaTeX rendering |
| Appearance/errors | [settings.rs](src/settings.rs), [theme.rs](src/theme.rs), [error.rs](src/error.rs): globals, theme mapping and safe error copy |

## State and interaction rules

- Keep the sidebar full height and place the titlebar only inside the main
  panel. It must never extend over the sidebar. The code workbench is split
  beside the conversation at sufficient width and takes the content area in
  narrow layouts.
- Keep GPUI `Task` handles with their owning view. Replaceable loads use
  generation checks so stale responses cannot overwrite a new conversation,
  project, or reopened tab. Do blocking work through injected async workflows.
- Conversation rendering retains at most 150 messages. Provider context comes
  from storage and must not be assembled from that visible slice. Terminal
  saves happen separately from streaming text updates; a failed save retains
  the message and blocks navigation until recovery.
- Activity sections follow active commands/tool work unless the user explicitly
  opens or closes them. Manual choices are session state. Approval controls
  remain accessible even when completed activity is collapsed.
- Opening a file creates or focuses its tab. Each tab retains file and diff
  state; a new change for that path can update its preview. Closing the active
  tab selects a neighbor. Changing conversations clears the workbench session.
- Editors are read-only. Diff mode displays agent-provided unified diffs with
  hunk navigation, not Git history or an editable merge surface. File mode can
  show proposed content while reviewing an uncommitted agent change. Diff
  snapshots and tabs are not restored from history after a session reset.

## Keyboard and appearance

| Action | Shortcut |
| --- | --- |
| Conversation finder | Ctrl+K; Cmd+K on macOS |
| Next / previous workbench tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Close active workbench tab | Ctrl+W; Cmd+W on macOS |
| Switch file/diff mode | Ctrl+Shift+D; Cmd+Shift+D on macOS |
| Next / previous diff hunk | F7 / Shift+F7 |

Workbench shortcuts are scoped to its key context. Diff actions need an active
diff; file-only tabs cannot switch to a nonexistent change view.

Light/dark surface and syntax colors live in
[themes/magenta.json](../../themes/magenta.json). Use theme tokens and semantic
status colors. Settings choose installed UI/mono font families and sizes; they
do not download fonts, and explicit component sizes remain part of typography.
Math has separate style and size settings. See the
[root settings reference](../../README.md#local-data-and-settings).

## Verify

From the repository root:

```bash
cargo test --locked -p magenta-ui --all-features
cargo doc --locked -p magenta-ui --all-features --no-deps
```

Tests use GPUI Kit's `test-support` feature and cover lifecycle, navigation,
composer validation, workbench state and recovery. Layout, focus, contrast,
system fonts, and real windows also need the manual checks in the
[development guide](../../docs/development.md). Error changes must follow the
[error-handling guide](../../docs/error-handling.md).
