# Changelog

## 0.1.0-alpha.1 — unreleased

This is the first experimental Alpha candidate for technical Linux users.
The release target is Ubuntu 24.04 on Wayland and X11. Packaging, release
automation, and the remaining Alpha exit checks are tracked in [ROADMAP.md](ROADMAP.md).

### Added

- Native Chat and Work conversations with streaming, cancellation, Markdown,
  code, math, and managed image attachments in Chat.
- Local conversation history, project registration, search, and response recovery.
- Workspace file previews, approvals, AgentFS staging, and final Apply/Discard.
- A read-only code workbench and Git status, diffs, staging, and confirmed commits.
- ChatGPT browser sign-in, live model discovery, separate Chat/Work defaults,
  and one-turn `/plan`, `/review`, and `/explain` commands.
- First-run setup guidance and persistent appearance, typography, and onboarding settings.
- An explicit Alpha support/privacy contract, contribution guide, and issue forms.

### Changed

- Refine the interface with graphite and ivory themes, a custom Magenta mark,
  quieter navigation, clearer conversation typography, and consistent surfaces.
- Add draft-only Chat/Work starting points and adapt the composer to narrow panes.
  Provide an opt-in native renderer for reproducible interface previews.
- Establish `0.1.0-alpha.1` across all workspace packages, with shared MIT
  licensing, repository metadata, Rust requirements, and disabled crates.io publication.
- Pin the development toolchain to Rust 1.99.1 and update direct dependencies
  to their current stable releases, including GPUI Kit 0.9.0. Retain Turso 0.4.4
  as a documented AgentFS compatibility exception.
- Check all workspace targets with strict Clippy and warning-free Rust
  documentation, and configure weekly Cargo/GitHub Actions dependency updates.

### Known limitations

- OpenAI through ChatGPT sign-in is the only production provider.
- Desktop command execution remains disabled while the reviewed AgentFS overlay
  cannot be mounted inside the command sandbox.
- Local data is unencrypted. Workbench tabs are session-local, and interrupted
  streaming can lose text that has not reached a terminal save.
- See [Alpha limitations and privacy](docs/alpha.md) before using real projects.
