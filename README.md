# Magenta

Magenta is an early-stage native AI workspace interface built in Rust with
[GPUI](https://gpui.rs/) and
[GPUI Component](https://github.com/longbridge/gpui-component).

The project is currently focused on building a polished desktop shell: custom
window chrome, a collapsible navigation sidebar, a conversation workspace,
JSON-backed themes, accessible prompt composition, and recoverable startup
behavior. Generated content, account controls, and service-backed destinations
are still visual prototypes; Magenta does not connect to an AI provider yet.

The visual direction is inspired by the
[Mogonta AI Chat Workspace UI design](https://dribbble.com/shots/27203662-Mogonta-AI-Chat-Workspace-UI-Design).
Magenta is an independent implementation and is not affiliated with the
original designer.

## What works today

- Native GPUI desktop window with custom Linux client-side chrome.
- Expanded and compact sidebar layouts with destination selection.
- Editable multiline prompt composer with native text input behavior.
- Combined model and effort menu with explicit generation readiness.
- Native reference-image selection with removable previews and safe validation.
- Bundled light and dark themes loaded from a JSON theme set.
- Safe fallback to GPUI Component's default dark theme when Magenta's theme
  cannot be loaded.
- Typed errors through `MagentaError` and the shared `Result<T>` alias.
- Persistent, deduplicated GPUI notifications for recoverable failures.
- Structured local diagnostics with daily rotation and stderr fallback.

## Requirements

- Rust 1.97.1 or newer. The locked GPUI revision uses Rust 1.97.1 upstream.
- Git and network access for the first dependency fetch.
- A Linux Wayland or X11 desktop with the native development libraries needed
  by GPUI. Linux is the platform currently exercised by this repository.

The dependency graph is captured in `Cargo.lock`. Use `--locked` for
reproducible development and verification.

## Run Magenta

From the repository root:

```bash
cargo run --locked
```

The first build compiles GPUI and its graphics stack, so it can take longer
than subsequent launches.

The repository also includes a [bacon](https://dystroy.org/bacon/) configuration:

```bash
bacon
bacon run-long
```

`bacon` is optional and must be installed separately.

## Development checks

Run the same widening verification gates used during development:

```bash
cargo fmt --all -- --check
cargo check --workspace --locked --all-targets
cargo test --workspace --locked
cargo clippy --workspace --locked --all-targets -- -D warnings
```

Compilation and tests do not replace launching the application. UI changes
should also be exercised in the real window at the default and compact sidebar
sizes.

## Project structure

```text
magenta/
├── crates/
│   ├── desktop/         # process startup, diagnostics, window configuration
│   └── ui/              # root view, components, themes, and UI error mapping
├── docs/
│   └── error-handling.md
├── themes/
│   └── magenta.json     # bundled GPUI Component light/dark theme set
├── Cargo.toml
└── bacon.toml
```

The `desktop` crate owns process and platform concerns. The `ui` crate owns the
GPUI view tree and exports `MainView`, theme operations, `MagentaError`, and the
project `Result<T>` type.

## Themes

Magenta's bundled themes live in [`themes/magenta.json`](themes/magenta.json).
They use GPUI Component's theme schema and provide `Magenta Light` and
`Magenta Dark` configurations.

Theme operations return typed results:

```rust
magenta_ui::theme::apply_named("Magenta Dark", cx)?;
magenta_ui::theme::toggle(cx)?;
```

An unavailable theme leaves the current theme unchanged. A malformed bundled
theme falls back to GPUI Component's default dark theme and produces a
user-safe warning.

## Errors and diagnostics

Application errors are represented by `MagentaError` and preserve their source
chains for developer diagnostics. User-facing messages are mapped separately
so raw paths, credentials, prompts, and internal details are not exposed.

Logs are written to the platform-local Magenta data directory under
`magenta/logs`. They rotate daily and retain seven files. If file logging is
unavailable, Magenta continues with stderr logging. Development verbosity can
be adjusted with `RUST_LOG`:

```bash
RUST_LOG=magenta=debug,gpui=info cargo run --locked
```

See [`docs/error-handling.md`](docs/error-handling.md) for the error presentation,
recovery, privacy, and future asynchronous-work conventions.

## Current roadmap

- Connect generation and chat actions to a model/service boundary.
- Add durable conversations, folders, and user preferences.
- Add explicit loading, empty, cancellation, retry, and offline states.
- Complete keyboard, screen-reader, resize, and cross-platform runtime QA.
- Add packaging, signing, and installed-artifact verification before release.

## Contributing

Keep changes scoped to the owning crate, preserve the existing state model,
and accompany behavioral changes with focused tests. Before handing off a
change, run the development checks above and launch the real application when
the change affects layout, interaction, themes, windows, or platform behavior.
