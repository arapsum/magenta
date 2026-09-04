# Magenta

Magenta is an experimental native AI chat client written in Rust with
[GPUI](https://gpui.rs/) and
[GPUI Component](https://github.com/longbridge/gpui-component).

Its premise is deliberately focused: provide a polished, local-first desktop
experience for remote AI providers without turning a chat client into a
browser tab in a desktop wrapper. Magenta is intended to be fast at idle,
careful with memory, and pleasant to use for long-lived conversations.

> **Status:** Magenta is in the interface-foundation stage. It does not yet
> connect to a remote AI provider or persist conversations. The current
> application is an interactive native shell with in-memory demo conversations
> and a deterministic provider adapter for shaping the chat experience. The
> first headless application workflow now prepares and starts a send operation
> independently of GPUI.

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
  - searchable representative conversation history;
  - pinned conversations and grouped recent history;
  - local pin/unpin, selection, expansion, and “Show more” interactions;
  - an active-selection bevel that moves from New Chat to the selected
    conversation;
  - a local profile/settings entry point and a theme toggle.
- Empty workspace with Magenta’s animated glass orb and ambient surface.
- Native multiline prompt composer with model/effort selection, image
-  attachment validation and previews, a disabled-until-ready submission
  state, and a circular send/stop control. This remains a UI prototype; it
  does not make network requests.
- A conversation surface backed by provider-independent core types and
  representative in-memory fixtures for every sidebar conversation.
- Conversation selection, new-thread creation, rich Markdown responses,
  code-block copy actions, response regeneration, cancellable demo streaming,
  provider failure handling, and safe local synchronization back into the demo
  catalog.
- A focused `SendMessage` application workflow that validates prompt history,
  allocates provider-neutral messages, derives new conversation titles, and
  invokes the provider without depending on GPUI. The UI still owns its
  transitional in-memory ID allocator while the demo catalog is active.
- A `ChatProvider` port in `core` with a deterministic `DemoProvider` adapter
  in `providers`; the UI consumes typed generation events rather than fake
  response content or provider-specific payloads.
- Typed errors through `MagentaError` and a shared `Result<T>` alias.
- Recoverable notifications and privacy-safe error presentation.
- Local structured diagnostics with daily rotation and stderr fallback.

## Not implemented yet

- Remote provider integrations or model discovery.
- Remote provider streaming, cancellation, retry, and usage accounting.
- SQLite conversation persistence, search indexing, or pagination.
- Secure credential storage.
- Durable conversation create, rename, delete, and search workflows.
- Attachment rendering and persistence beyond the present composer prototype.
- Rich provider events such as reasoning, tool calls, citations, and usage.

## Planned architecture

The workspace intentionally starts small:

```text
crates/
├── application # headless application workflows such as sending a message
├── core        # provider-independent values, ports, events, and errors
├── desktop     # executable, diagnostics, platform/window composition
├── providers   # provider adapters; currently the deterministic demo provider
└── ui          # GPUI app shell, components, themes, and demo views
```

The current dependency direction is intentionally small:

```text
desktop
├── application
│   └── core
├── providers
│   └── core
└── ui
    ├── application
    │   └── core
    └── core
```

The demo conversation catalog and its transitional ID allocator currently live
in `ui` so the interaction can be designed with fake data. The provider port
and generation events live in `core`, while `providers` owns the deterministic
adapter that supplies the current local stream. The `application` crate now
owns the provider-facing preparation for “send message”: it receives domain
values, returns a pending generation, and leaves stream lifecycle and rendering
to the UI. Regeneration remains in the UI until its workflow is large enough to
extract cleanly. Future storage and remote-provider adapters will depend on
`core`; `core` must not depend on GPUI, HTTP, SQLite, or a specific model API.

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
small window size.

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

1. Complete visual and keyboard/accessibility QA for the conversation surface.
2. Move regeneration orchestration into the application workflow boundary.
3. Implement one streaming remote provider behind the existing port.
4. Add local SQLite persistence with a lightweight conversation index and
   paged message loading.
5. Add provider settings, secure credential storage, and durable attachments.

## Contributing

Keep changes scoped to the owning crate and preserve the application’s clean
dependency direction. Add focused tests for behavioral changes, run the checks
above, and launch the real application for changes that affect layout,
interaction, themes, windows, or platform behavior.
