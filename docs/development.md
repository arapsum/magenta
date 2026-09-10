# Development guide

Run commands below from the repository root. For component ownership and
current product behavior, start with the [project README](../README.md).

## Toolchain and Linux dependencies

[CI](../.github/workflows/ci.yml) uses Rust **1.98.0** on Ubuntu 24.04. This is the
tested toolchain, not a separately maintained minimum supported Rust version.
Most crates use edition 2024; the desktop package uses edition 2021. Keep
`Cargo.lock` and use `--locked` to reproduce the dependency graph.

GPUI Kit is pinned in the [workspace manifest](../Cargo.toml). Its native
graphics stack requires system development libraries. The Ubuntu packages used
by CI are:

```bash
sudo apt-get update
sudo apt-get install --yes \
  build-essential clang cmake libasound2-dev libdbus-1-dev \
  libfontconfig-dev libgit2-dev libglib2.0-dev libsqlite3-dev libssl-dev \
  libva-dev libvulkan1 libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev \
  libzstd-dev lld pkg-config
```

Package names differ across distributions. Running the app also needs a usable
Wayland or X11 desktop and graphics drivers. On Linux, provider credentials use
the desktop Secret Service through `keyring`; a working, unlocked service is
needed for sign-in persistence.

Bubblewrap is optional for building and chatting. Install `bubblewrap` and
enable the necessary user namespaces to exercise Agent commands. The startup
probe decides whether commands are available; the app can run with file tools
only. See [workspace documentation](../crates/workspace/README.md).

## Run and inspect

```bash
cargo run --locked -p magenta-desktop
```

The binary is named `magenta`. Normal launches use real local history and
settings; there is no default demo history or mock account. `DemoProvider` is
available to tests and other callers, but is not selected by a desktop CLI flag.

For more local diagnostics:

```bash
RUST_LOG=magenta=debug,magenta_ui=debug,magenta_providers=debug,gpui=info cargo run --locked
```

Logs are written to stderr and the platform-local `magenta/logs` directory,
with daily rotation and seven retained files. If file logging fails, stderr is
the fallback. Review logs before sharing them: technical source errors may
contain local details. See [error handling](error-handling.md).

The optional [bacon configuration](../bacon.toml) supports `bacon` for checking
and `bacon run-long` for launch/restart. Its default jobs generally target the
default package; use the commands below for workspace-wide verification.

## Automated checks

```bash
cargo fmt --all -- --check
cargo check --workspace --locked --all-targets --all-features
cargo test --workspace --locked --all-features
cargo clippy --workspace --locked --all-targets --all-features -- \
  -D warnings -W clippy::pedantic -W clippy::nursery -W rust-2018-idioms
```

`default-members = ["crates/desktop"]` means an unqualified `cargo test`, even
with `--all-targets`, selects the desktop package's tests. Use `--workspace` for
all packages, or `-p magenta-storage` (for example) for a focused check. CI runs
workspace tests; its Clippy command uses default package selection. The command
above explicitly lints all workspace targets, including library test targets.

The crate READMEs list focused tests. GPUI tests use the kit's test-support
feature. Real command-isolation tests in `magenta-workspace` return early when
Bubblewrap or its isolation probe is unavailable; a passing run on such a host
does not establish that command execution works there.

## Documentation

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --all-features --no-deps
```

Generated pages are under `target/doc/`, including
`target/doc/magenta_core/index.html` and `target/doc/magenta/index.html` for the
desktop binary. Do not commit the generated output.

When changing a public contract, update its Rust doc comments and its crate
README. When changing user behavior or defaults, update the project README.
Keep command examples, limits, schema versions, and dependency ownership tied
to the source rather than listing planned features as implemented.

## Manual UI review

Exercise the states affected by the change, including:

- New Chat, a project landing page, and long conversations with paged history.
- Expanded/collapsed sidebar, narrow window, and open/closed code workbench.
  The titlebar must stay within the main panel, beside the full-height sidebar.
- File tabs, duplicate filenames, explorer collapse, diff/file switching,
  hunk navigation, file-load failure, and conversation switches.
- Streaming, Stop, failed response recovery, failed-save retry, and tool
  approval/denial. Manually opened activity sections should respect the user's
  choice while automatic sections collapse when inactive.
- Keyboard navigation, visible focus, light/dark/system themes, and typography.
  Check long text and dense code, not only the empty screen.
- The separate Settings window: open/close/minimize, reload valid and invalid
  TOML, and restore defaults. Use disposable configuration data for destructive
  reset or deletion checks.

## Contribution boundaries

Keep workflows in `application`, contracts in `core`, provider wire formats in
`providers`, persistence in `storage`, filesystem/process policy in `workspace`,
and view state in `ui`. `desktop` composes those pieces. Add focused behavioral
tests for changed contracts and failure paths. Avoid making the UI depend on
concrete storage or provider implementations.
