# Contributing to Magenta

Magenta is preparing its first experimental Linux Alpha. Choose work from
[ROADMAP.md](ROADMAP.md), and keep changes focused enough to review and validate.
The supported release target is Ubuntu 24.04 on Wayland and X11; see the
[Alpha contract](docs/alpha.md) for scope and release blockers.

## Set up

Install Rust with rustup and the native libraries listed in the
[development guide](docs/development.md). `rust-toolchain.toml` selects the
reviewed stable toolchain and installs its rustfmt and Clippy components.

```bash
cargo run --locked
```

Normal launches use your real local settings, history, and account. Use
disposable projects and separate `XDG_DATA_HOME` / `XDG_CONFIG_HOME` directories
when testing migrations, deletion, settings reset, and agent mutations. Tests
should use temporary directories and fake providers; never require live credentials.

## Ownership and conventions

Keep domain contracts in `core`, workflows in `application`, provider protocols
in `providers`, persistence in `storage`, filesystem and process policy in
`workspace`, view state in `ui`, and composition in `desktop`. The
[crate guides](README.md#documentation-and-crate-ownership) describe each boundary.

- Preserve typed failures and safe recovery copy; follow
  [error handling](docs/error-handling.md).
- Keep blocking I/O off the GPUI thread and retain task handles with their owner.
- Keep provider context independent of the rendered history page.
- Require previews and approvals for agent mutations, and preserve final
  AgentFS review. Read-only commands must enforce their allowlist in code.
- Keep the sidebar full height, with the titlebar contained in the main panel.
- Target at most 750 lines per Rust module; CI rejects modules over 800 lines.
  Directory modules use `mod.rs` without a sibling registration file.
- Update Rust docs and crate guides with public contracts, and user documentation
  with behavior, limits, data locations, and defaults.

## Validate a change

Run focused behavioral tests while developing, then the applicable workspace checks:

```bash
cargo fmt --all -- --check
bash scripts/check-rust-module-lines.sh
cargo check --workspace --locked --all-targets --all-features
cargo test --workspace --locked --all-features
cargo clippy --workspace --locked --all-targets --all-features -- \
  -D warnings -W clippy::pedantic -W clippy::nursery -W rust-2018-idioms
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --all-features --no-deps
```

UI changes also need the real-window checks in the development guide. Record
which checks passed and any environment limitations in your pull request.
Bubblewrap tests may skip when isolation is unavailable; distinguish that
from tested command execution.

## Update Rust and dependencies

Use current stable releases. Check the official Rust release announcements and
crates.io, update `rust-toolchain.toml`, the workspace `rust-version`, CI, and
the documented baseline together, then regenerate `Cargo.lock` with `cargo update`.
Adapt breaking APIs and run the affected behavioral tests. Keep GPUI Kit and
its direct `gpui-pre-*` adapters on matching versions. Upstream dependencies
can retain older transitive versions; document any direct compatibility pin.
Do not upgrade to nightly or prerelease dependencies solely because their
version numbers are higher.

## Report and review

Use the repository's issue forms for crashes, provider failures, agent behavior,
and packaging. Include reproduction steps, the exact version, desktop session,
expected behavior, and validation results. Inspect diagnostic excerpts before
sharing them; omit credentials, private prompts, protected file contents, and
private workspace data.

Pull requests should explain the triggering problem, resulting behavior, and
verification. Known data-loss, authentication, sandbox-escape, and crash-on-launch
defects block an Alpha release. Release status and remaining evidence belong in
the roadmap; adding a version number alone does not make a release ready.
