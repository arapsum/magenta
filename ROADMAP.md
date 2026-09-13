# Roadmap to Magenta 0.1.0 Alpha

Magenta's first public Alpha will be an explicitly experimental GitHub
prerelease for technical Linux users. It will support Ubuntu 24.04 on Wayland
and X11, use OpenAI as its sole production provider, and establish extensible
contracts for future providers.

The Alpha is intended to turn the current prototype into an installable,
recoverable, and testable release without prematurely expanding provider or
platform support.

## Milestone 1: Define the Alpha contract

- Establish `0.1.0-alpha.1` as the first Alpha version.
- Add complete Cargo package metadata, including the MIT license, repository,
  description, Rust version, and publishing policy.
- Document the supported environment:
  - Ubuntu 24.04 on Wayland and X11.
  - ChatGPT browser authentication and Secret Service keyring integration.
  - Bubblewrap is optional for Chat and file-only Work mode, but required for
    agent commands.
- Add a changelog, contribution guidance, issue templates, and an Alpha
  limitations and privacy document.
- Treat known data-loss, authentication, sandbox-escape, and crash-on-launch
  defects as release blockers.

## Milestone 2: Add per-mode model defaults

- Let users configure separate default models and reasoning efforts for Chat
  and Work.
- Persist provider-aware generation preferences in the TOML settings file.
- Advance the settings schema to version 2 while retaining version-1
  compatibility, comments, and unknown keys.
- Add a **Models and defaults** page to Settings, populated from the live model
  catalog.
- Resolve a generation configuration in this order:
  1. Existing conversation or explicit retry configuration.
  2. The user's Chat or Work default.
  3. The provider-recommended model and effort.
  4. The first available catalog model and its default effort.
- If a saved model or effort is no longer available, use the provider fallback
  and show a non-blocking notice linking to Settings.
- Keep composer selections local to the current conversation; selecting a model
  in the composer must not silently replace the saved default.

The settings contract should use provider-neutral values equivalent to:

```rust
struct GenerationPreference {
    provider: ProviderId,
    model: ModelId,
    effort: EffortLevel,
}

struct GenerationSettings {
    chat: Option<GenerationPreference>,
    work: Option<GenerationPreference>,
}
```

## Milestone 3: Implement typed provider commands

- Introduce provider-neutral command identifiers, descriptors, supported modes,
  response instructions, and agent tool policies.
- Add a command-catalog port implemented initially by the OpenAI provider.
  Future providers should be able to advertise different commands without
  adding provider checks to the UI.
- Extend turn and generation requests with an optional typed command selection.
  Persist the selection so retries and regeneration reproduce the same
  behavior.
- Open a searchable command palette when `/` is typed at the beginning of the
  composer. Support keyboard navigation, descriptions, availability states,
  and a removable selected-command chip.
- Apply a command to one submission only and clear it after submission.
- Ship these commands in the Alpha:
  - `/plan`: Work-only. Inspect the workspace with read-only tools and return an
    implementation plan.
  - `/review`: Work-only. Inspect files and repository status or diffs, then
    return findings before summaries.
  - `/explain`: Available in Chat and Work. Work may inspect workspace files,
    but the command remains read-only.
- Enforce read-only behavior in the application layer by withholding mutation
  and command-execution tools, rather than relying only on prompt instructions.
- Expose repository status and diff as read-only agent tools. Do not add branch
  operations or mutating Git tools.

## Milestone 4: Harden the public Alpha experience

- Add first-run guidance for sign-in, model availability, Chat versus Work,
  workspace selection, and Bubblewrap capability.
- Add an **About and diagnostics** surface showing the application version,
  supported platform, local data locations, and actions to open logs or
  settings and copy a sanitized diagnostic summary.
- Keep diagnostics local; do not add telemetry or automatic uploads.
- Distinguish offline, timeout, rate-limit, provider-service, account, and model
  access failures with relevant recovery actions.
- Honor provider retry guidance without retrying non-idempotent agent actions
  automatically.
- Recover cleanly from expired authentication, interrupted browser sign-in,
  empty or changing model catalogs, malformed settings, unavailable Bubblewrap,
  failed persistence, and interrupted agent runs.
- Preserve partial responses and make retry behavior explicit to the user.
- Test live model-catalog changes while Settings or an existing conversation is
  open.

## Milestone 5: Secure user data and agent boundaries

- Write a threat model covering browser authentication and its local callback,
  keyring storage, provider requests, workspace isolation, symlink and path
  escapes, prompt injection, tool approvals, Git access, and diagnostic output.
- Add `SECURITY.md` with the supported-version and private vulnerability-reporting
  policy.
- Verify that logs and diagnostic summaries never include access tokens,
  protected-file contents, environment secrets, or full prompts by default.
- Audit dependencies and generate a software bill of materials for every
  release artifact.
- Back up the conversation database before irreversible schema migrations and
  retain the original if a migration fails.
- Test abrupt shutdown during streaming, settings saves, attachment management,
  database migration, and agent mutations.
- Add a portable conversation export before exposing destructive local-data
  clearing. Import may remain a post-Alpha feature.
- Document local-data retention, backup, downgrade, and migration behavior.
- Provide a confirmed workflow for clearing Magenta's local data. Workspace
  files and provider credentials must remain untouched unless explicitly
  selected.

## Milestone 6: Set performance and accessibility baselines

- Define measurable Alpha budgets for startup time, idle memory, streaming
  responsiveness, and composer input latency.
- Benchmark long Markdown and code responses, thousands of conversations,
  large repositories, large files and diffs, and extended agent sessions.
- Preserve bounded memory behavior in conversation history, tool output, file
  viewing, search, and diff rendering.
- Complete keyboard-only navigation and visible focus coverage for the composer,
  command palette, Settings, approvals, Files and Changes selectors, and
  dialogs.
- Give every interactive control an accessible name and verify screen-reader
  output for important state changes and errors.
- Test at 100%, 150%, and 200% display scaling, and validate high-contrast text
  and controls in light and dark themes.
- Respect reduced-motion preferences by disabling or simplifying animated
  background effects.
- Verify text selection, clipboard operations, IME input, and layouts containing
  long labels or translated text.

## Milestone 7: Package, release, and collect feedback

- Produce an Ubuntu 24.04 `.deb` containing the application icon, desktop entry,
  and declared native runtime dependencies.
- Treat Bubblewrap as an optional or recommended dependency and explain the
  resulting capability difference in the application.
- Ensure ordinary installation, upgrade, and uninstall operations preserve user
  data.
- Add a tag-driven GitHub release workflow that:
  - Runs formatting, workspace tests, strict Clippy, `cargo check`, and
    warning-free Rust documentation.
  - Builds the release binary and `.deb` on Ubuntu 24.04.
  - Publishes the package, SHA-256 checksum, software bill of materials,
    changelog excerpt, third-party notices, and known limitations.
- Display the exact application version in Settings or About.
- Use manual GitHub downloads for Alpha updates. Update notifications and
  automatic installation are deferred.
- Add an in-app **Report a problem** action that opens a prefilled GitHub issue
  without uploading data automatically.
- Let users inspect and copy the sanitized diagnostic summary before sharing
  it.
- Add issue templates for crashes, provider failures, agent behavior, and
  packaging problems, along with a documented triage policy.
- Document how a faulty release is withdrawn and rehearse restoring the previous
  `.deb` without losing settings, conversations, or attachments.
- Define the evidence required to leave Alpha, including stability feedback from
  real installations and no unresolved release-blocking defects.

## Alpha exit criteria

- Settings tests cover version-1 migration, independent Chat and Work defaults,
  unknown-key preservation, invalid effort values, and missing-model fallback.
- Command tests cover parsing, keyboard selection, mode restrictions, one-turn
  clearing, persistence, retries, provider instructions, and read-only tool
  enforcement.
- Provider contract tests verify command catalogs and OpenAI wire requests
  without requiring live credentials.
- Security tests cover callback validation, credential redaction, protected
  paths, symlink escapes, prompt-injected tool requests, and read-only command
  enforcement.
- Migration tests cover successful upgrades, failed-migration rollback,
  database backups, abrupt shutdown recovery, and downgrade documentation.
- Performance checks record the agreed startup, memory, streaming, history,
  repository, and long-session baselines and flag material regressions.
- Accessibility checks cover keyboard-only operation, screen-reader labels,
  reduced motion, contrast, display scaling, clipboard use, and IME input.
- Release smoke tests cover clean `.deb` installation, launch, upgrade, and
  uninstall on Ubuntu 24.04, plus rollback to the preceding package.
- Manual Wayland and X11 checks cover authentication, streaming, cancellation,
  history recovery, image attachments, Work tools, approvals, Git Changes,
  Settings, and environments both with and without Bubblewrap.
- Formatting, workspace tests, strict Clippy, `cargo check`, and documentation
  checks pass.
- No known critical or high-severity release blockers remain.

## After the Alpha

- Add Anthropic as the second production provider to validate the provider,
  authentication, model-default, and command abstractions.
- Add in-app update notifications before considering an automatic updater.
- Expand packaging based on tester feedback, beginning with AppImage or Flatpak
  and then additional Linux distributions.
- Evaluate conversation import, exact tokenization and context summarization,
  restored workbench sessions, and richer reasoning or citation events for the
  Beta roadmap.

## Alpha boundaries

- OpenAI remains the only production provider.
- Commands are typed Magenta capabilities advertised by providers, not raw
  slash-text passthrough.
- `/plan`, `/review`, and `/explain` apply to one submission.
- Ubuntu 24.04 is the only supported platform.
- Telemetry, automatic updates, network-enabled workspace commands, and
  additional providers are out of scope.
