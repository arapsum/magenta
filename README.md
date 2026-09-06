# Magenta

Magenta is an experimental native AI chat client written in Rust with
[GPUI](https://gpui.rs/) and
[GPUI Component](https://github.com/longbridge/gpui-component).

Its premise is deliberately focused: provide a polished, local-first desktop
experience for remote AI providers without turning a chat client into a
browser tab in a desktop wrapper. Magenta is intended to be fast at idle,
careful with memory, and pleasant to use for long-lived conversations.

> **Status:** Magenta supports ChatGPT sign-in, OpenAI streaming, local SQLite
> conversation history, and an editable TOML-backed settings window.
> Conversations survive restarts, including their model, effort, pinned state,
> and completed, stopped, or failed responses. The application remains
> experimental; Linux is the platform exercised here.

The visual language takes inspiration from the
[Mogonta AI Chat Workspace UI design](https://dribbble.com/shots/27203662-Mogonta-AI-Chat-Workspace-UI-Design).
Magenta is an independent implementation and is not affiliated with the
original designer.

## Product direction

Magenta is a general AI chat client, not an image studio or an agent/terminal
workstation. Its eventual core experience is:

```text
Native GPUI interface
        |
        v
Conversation workflows
        |
        +---- provider adapters ----> remote model APIs
        |
        +---- local persistence ----> SQLite + local attachment files
```

The product is guided by a few constraints:

- **Native all the way down.** The chat experience should not require an
  embedded Chromium, Electron, or WebView runtime.
- **Provider-neutral domain model.** The application should model
  conversations, messages, attachments, and generation events—not one
  provider’s request JSON.
- **Local-first history.** Conversations and settings belong on the user’s
  machine; credentials should use the operating system’s secure credential
  store rather than a plaintext database.
- **Bounded working set.** Memory should scale chiefly with what is visible:
  lightweight conversation summaries in the sidebar, paged messages in an
  open thread, and released attachment buffers when they are no longer needed.
- **Streaming without churn.** Provider stream chunks should update the active
  message at a sensible visual cadence rather than rebuilding an entire
  conversation for every token.

## What works today

- Native GPUI window with custom Linux client-side chrome.
- Bundled JSON light and dark themes, including safe fallback behavior if a
  bundled theme cannot be loaded.
- A responsive, collapsible chat sidebar with:
  - New Chat state;
  - searchable persisted conversation titles;
  - pinned conversations and grouped recent history;
  - durable pin/unpin, inline rename, selection, expansion, and “Show more”
    interactions;
  - permanent conversation deletion behind a confirmation dialog;
  - an active-selection bevel that moves from New Chat to the selected
    conversation;
  - a local account menu with profile details, settings, theme switching,
    provider connection, and sign-out actions.
- Empty workspace with Magenta’s animated glass orb and ambient surface.
- Native multiline prompt composer with model-specific effort selection,
  attachment validation and previews, fenced-code previews, and a circular
  send/stop control. OpenAI currently accepts text messages only.
- ChatGPT browser sign-in, OS-keyring credentials, account restoration,
  model discovery, and real OpenAI response streaming.
- A separate settings window with System/Light/Dark appearance modes, UI and
  monospace font families and sizes, KaTeX math styles and sizes, OpenAI
  connection controls, and local configuration-file actions.
- A conversation surface backed by provider-independent core types and SQLite.
- Conversation selection, new-thread creation, rich Markdown responses,
  code-block copy actions, response regeneration, cancellable streaming,
  provider failure handling, and per-response model attribution.
- Focused `SendMessage` and `RegenerateMessage` application workflows that
  commit message turns before invoking the provider, loading complete context
  from storage independently of the visible message page.
- `ChatProvider` and `ConversationStore` ports in `core`, implemented by the
  provider and storage crates. A deterministic `DemoProvider` remains available
  for tests; production history starts empty.
- Asynchronous history loading, 50-message pages with “Load earlier messages,”
  preserved scroll position, and recoverable save failures.
- Generation completion metadata on assistant messages, including normalized
  finish reasons and optional input/output token usage. GPUI owns the active
  stream task, so cancelling or superseding a response drops the stream and
  stale chunks cannot alter the conversation.
- Workflow-specific application errors plus a typed `MagentaError` and
  `Result<T>` alias at the UI boundary.
- Recoverable notifications and privacy-safe error presentation.
- Local structured diagnostics with daily rotation and stderr fallback.

## Not implemented yet

- Additional provider integrations.
- Full-text search indexing and automatic context-budget management.
- Attachment rendering and persistence beyond the present composer prototype.
- Rich provider events such as reasoning, tool calls, and citations.

## Architecture

The workspace intentionally starts small:

```text
crates/
├── application # headless application workflows such as sending a message
├── core        # provider-independent values, ports, events, and errors
├── desktop     # executable, diagnostics, platform/window composition
├── providers   # OpenAI adapter and deterministic test provider
├── storage     # SQLite adapter, migrations, and record conversion
└── ui          # GPUI app shell, components, themes, and active conversation
```

The current dependency direction is intentionally small:

```text
desktop
├── application
│   └── core
├── providers
│   └── core
├── storage
│   └── core
└── ui
    ├── application
    │   └── core
    └── core
```

`desktop` constructs the adapters and injects them into application workflows.
`application` coordinates durable turns and provider requests; `ui` owns stream
lifecycle, cancellation, loaded pages, and rendering. SQLite allocates IDs and
message sequences. The `SettingsStore` port keeps settings persistence behind
the same boundary: `storage` provides the TOML adapter and `ui` applies the
active settings to the running windows. `core` has no dependency on GPUI, HTTP,
or SQLite.

## Local conversation history

History is stored at `<platform-local-data>/magenta/conversations.sqlite3`.
On Linux this is normally `~/.local/share/magenta/conversations.sqlite3`, with
`XDG_DATA_HOME` respected. The database contains plaintext message content,
generation configuration, usage metadata, timestamps, pins, and attachment
references. Credentials remain in the OS keyring. Attachment references do not
copy or preserve the original files, and missing files do not block loading.

SQLite uses bundled native libraries, WAL mode, foreign keys, a five-second
busy timeout, and transactional schema migrations. Blocking database work runs
off the GPUI thread. A send commits the user message and an assistant placeholder
before the provider starts. Completion, stop, and failure each save the final
response; individual streamed chunks do not cause database writes.

Normal window closure waits for a pending response save. After an abrupt exit,
unfinished placeholders recover as stopped; text streamed since the turn began
may be lost. Failed saves retain the visible response and offer Retry before
navigation. Existing in-memory demo history is not imported.

The sidebar loads lightweight summaries and filters titles locally. Conversation
titles can be renamed inline from a row’s overflow menu; Enter or focus loss
saves a non-empty changed title, while Escape cancels. Renaming preserves the
conversation’s recency and pin state. The same menu can permanently delete a
conversation after confirmation; this removes its SQLite records and attachment
references, but never deletes the original files. Opening a thread loads its
latest 50 messages. Earlier pages load on demand, and leaving a thread releases
its rendered messages. Provider context currently includes all
completed messages preceding the response, even when they are outside the
loaded page; token-budget truncation and page eviction are future work.

## Application settings

Magenta stores user preferences in an editable TOML file at
`<platform-config>/magenta/settings.toml`. On Linux this is normally
`~/.config/magenta/settings.toml`, with `XDG_CONFIG_HOME` respected. The file is
opened from the sidebar account menu by selecting **Settings**.

The settings window applies changes immediately and persists them without
blocking the GPUI thread. It currently provides:

- System, Light, and Dark appearance modes;
- UI and monospace font families, including installed font names and the
  `system-ui`/`system-monospace` defaults;
- UI and monospace font sizes;
- KaTeX Default, Roman, Sans-serif, and Typewriter math styles;
- inline and display-math sizes; and
- OpenAI account connection and disconnection controls.

The file uses the following shape. Values not listed here are preserved when
Magenta updates a setting, so comments and application-specific TOML keys can
remain in the file:

```toml
version = 1

[appearance]
theme = "dark"

[typography]
ui_font = "system-ui"
ui_size = 15
monospace_font = "system-monospace"
monospace_size = 13
math_font = "default"
inline_math_size = 13
display_math_size = 16
```

**Reload from disk** applies edits made outside Magenta after a successful
parse. **Restore defaults** copies the current file to a timestamped
`settings.toml.bak-*` sibling before writing the default values. Missing files
start from defaults. Provider credentials are never written to this file; they
remain in the operating system keyring.

## Requirements

- Rust 1.97.1 or newer; the locked GPUI revision currently requires it.
- Git and network access for the first dependency fetch.
- A Linux Wayland or X11 desktop with the native development libraries needed
  by GPUI. Linux is the platform currently exercised by this repository.

The dependency graph is committed in `Cargo.lock`. Use `--locked` for
reproducible development and verification.

## Run Magenta

From the repository root:

```bash
cargo run --locked
```

The first build compiles GPUI and its graphics stack, so it can take longer
than subsequent launches.

The repository also includes a [bacon](https://dystroy.org/bacon/) setup:

```bash
bacon
bacon run-long
```

`bacon` is optional and must be installed separately.

## Development checks

Run formatting, the workspace test suite, and the project’s strict lint gate:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked --all-features
cargo clippy --all-targets --all-features -- \
  -D warnings -W clippy::pedantic -W clippy::nursery -W rust-2018-idioms
```

Compilation and unit tests do not replace launching the application. UI changes
should also be exercised in the real window, including expanded and collapsed
sidebar states, keyboard focus, search, selection, theme switching, and a
small window size. Settings changes should additionally exercise opening,
minimizing, and closing the separate settings window, changing typography,
reloading the TOML file, and restoring defaults.

## Themes

Magenta’s bundled themes live in [`themes/magenta.json`](themes/magenta.json).
They use the GPUI Component schema and provide `Magenta Light` and `Magenta
Dark` configurations.

Theme operations return typed results:

```rust
magenta_ui::theme::apply_named("Magenta Dark", cx)?;
magenta_ui::theme::toggle(cx)?;
```

An unavailable theme leaves the active theme unchanged. A malformed bundled
theme falls back to GPUI Component’s default dark theme and presents a safe
warning.

## Errors and diagnostics

Application failures use `MagentaError`, retaining technical source chains for
diagnostics while mapping user-facing messages separately. Raw paths,
credentials, prompts, clipboard data, and internal details must not appear in
the interface.

Logs are stored in the platform-local Magenta data directory under
`magenta/logs`, rotate daily, and retain seven files. If file logging is not
available, Magenta continues with stderr logging. Development verbosity can be
adjusted with `RUST_LOG`:

```bash
RUST_LOG=magenta=debug,gpui=info cargo run --locked
```

See [`docs/error-handling.md`](docs/error-handling.md) for presentation,
recovery, privacy, and asynchronous-work conventions.

## Roadmap

1. Continue visual and keyboard/accessibility QA for the conversation surface.
2. Add full-text history search.
3. Bound provider context and evict distant loaded message pages.
4. Add durable attachments and additional providers.

## Contributing

Keep changes scoped to the owning crate and preserve the application’s clean
dependency direction. Add focused tests for behavioral changes, run the checks
above, and launch the real application for changes that affect layout,
interaction, themes, windows, or platform behavior.
