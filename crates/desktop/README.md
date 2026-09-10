# magenta-desktop

The executable and composition root for Magenta. The Cargo package is
`magenta-desktop`; its binary is **`magenta`**. It is the workspace's default
member. This package currently uses edition 2021, while library crates inherit
edition 2024. [Workspace overview](../../README.md).

## Startup and service wiring

[src/main.rs](src/main.rs) initializes diagnostics before fallible application
setup, creates the GPUI Kit application and asset source, initializes settings
and themes, and opens the main window.

It constructs and injects:

| Adapter/workflow | Used for |
| --- | --- |
| `OpenAiProvider` | Shared chat, agent, authentication and model-catalog ports |
| `SqliteConversationStore` | Conversation history and project registration |
| `TomlSettingsStore` | Persistent appearance/typography preferences |
| `LocalWorkspace` | Workspace tools and read-only project browsing |
| Optional `BubblewrapCommandRunner` | Commands, when discovery and isolation probing succeed |
| `SendMessage`, `RegenerateMessage`, `RunWorkspaceAgent`, `ConversationHistory`, `ProjectCatalog` | Workflows passed into the UI |

The startup paths are selected here using platform data/config directories.
The UI performs asynchronous account/history/settings loading after composition.
Missing command capability results in files-only Agent mode, rather than an
unrestricted command runner. Failure to create the main window exits with a
failing status; recoverable theme/diagnostics failures can use fallbacks and
present a warning once the window exists.

## Platform, assets, and diagnostics

Linux window options configure the custom client-side chrome. Internal layout,
including the rule that the titlebar stays beside the full-height sidebar,
belongs to [magenta-ui](../ui/README.md).

`MagentaAssets` embeds custom icons from [assets/icons](assets/icons) and the
workspace [assets/icons](../../assets/icons), then falls back to GPUI Kit's
assets. Theme JSON is embedded by `magenta-ui`. Asset registration must include
both loading and listing when adding application-owned icons.

[src/diagnostics.rs](src/diagnostics.rs) configures tracing, stderr output, daily
file rotation and seven retained log files. The process keeps the non-blocking
writer guard alive until shutdown. A panic hook records diagnostics and then
delegates to the prior panic handler. There is no built-in telemetry upload.

## Run and verify

From the repository root:

```bash
cargo run --locked -p magenta-desktop
cargo test --locked -p magenta-desktop
cargo doc --locked -p magenta-desktop --no-deps
```

The current unit tests check diagnostics configuration. They do not launch a
real GUI or test every library crate. The
[development guide](../../docs/development.md) lists workspace test commands,
Linux dependencies, and manual window checks. Launching the binary uses the user's normal local
database/settings and can restore their provider account.

Keep this crate focused on startup and wiring. New persistence/provider logic
belongs in its adapter crate, workflows in `application`, and interaction or
layout changes in `ui`.
