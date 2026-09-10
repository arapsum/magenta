# Error handling and recovery

Magenta separates technical errors, durable failure metadata, and user-facing
recovery. The UI's `Result<T>` alias uses `MagentaError`; it is not the result
type for every crate. Core ports expose their own typed failures, application
workflows add operation context, and the UI chooses the presentation.

## Ownership

| Layer | Types and responsibility |
| --- | --- |
| [Core](../crates/core/README.md) | `ProviderError`/`ProviderErrorKind`, `StorageError`, settings/workspace errors, and persisted `MessageFailure` |
| [Application](../crates/application/README.md) | `SendMessageError`, `RegenerateMessageError`, `RetryMessageError`, title/project errors; classify agent loop guards |
| [Adapters](../README.md#documentation-and-crate-ownership) | Preserve the source failure and map it to the appropriate core category |
| [UI](../crates/ui/src/error.rs) | `MagentaError`, `ErrorPresentation`, notifications, inline state and recovery actions |
| [Desktop](../crates/desktop/src/diagnostics.rs) | Diagnostics initialization, file/stderr sinks and panic handling |

Add a typed variant when a new failure needs a distinct recovery decision.
Preserve the source with `#[source]` or an unambiguous conversion. Do not hide
it in a catch-all string merely to make propagation compile.

## Presentation policy

| Failure | Presentation and recovery |
| --- | --- |
| Invalid prompt, attachment, or oversized newest turn | Inline composer error; retain input for correction |
| History initialization/load | Contextual unavailable/retry state; retain prior usable content where possible |
| Terminal response save | Keep visible text and offer Retry; defer navigation/close until the save succeeds |
| Provider generation | Persistent error on the assistant message, partial output, relevant action and collapsed technical details |
| Account/model discovery | Safe account/settings status with reconnect or reload guidance |
| File/tree load | Workbench failure state with Retry; distinguish missing, inaccessible, oversized and unsupported documents |
| Other failed user action | Operation-specific inline status or persistent notification; repeated notifications replace the same stable ID |
| Recoverable startup issue | Safe fallback plus warning after the window opens |
| Main window cannot open | Local diagnostics and failing exit status |
| Invariant violation | Panic diagnostics followed by normal panic handling |

Use `MagentaError::presentation` and `provider_error_presentation` for UI-boundary
failures. Generation failure cards are mapped separately in
[conversation rendering](../crates/ui/src/components/conversation/rendering.rs).
Do not display a raw `Display`/`Debug` source as normal recovery copy. Intentional
file paths in the explorer, command output, or project selection are different
from accidentally exposing technical error payloads.

## Durable generation failures

[`MessageFailure`](../crates/core/src/message.rs) stores a category, stable
reference code, provider ID, and optional allowlisted detail. Schema v7 stores
it alongside the assistant response. Allowed technical details are HTTP status
and observed/permitted agent round/tool-call counts; source errors, provider
response bodies, prompts, and credentials are not serialized into it.

The conversation card preserves partial output, exposes technical details on
request, and copies only that safe diagnostic representation. Existing failed
messages without structured metadata retain a generic fallback; migration
cannot reconstruct their original cause.

| Category | Reference | Primary recovery |
| --- | --- | --- |
| Authentication | `MAG-GEN-AUTH` | Open provider settings |
| Permission/model access | `MAG-GEN-PERMISSION` | Choose another model |
| Rate limit | `MAG-GEN-RATE-LIMIT` | Retry after waiting |
| Connection | `MAG-GEN-CONNECTION` | Retry |
| Service unavailable | `MAG-GEN-SERVICE` | Retry |
| Invalid request | `MAG-GEN-INVALID-REQUEST` | Focus composer to adjust request |
| Context | `MAG-GEN-CONTEXT` | Focus composer to reduce request |
| Incomplete response | `MAG-GEN-INCOMPLETE` | Retry |
| Agent ceiling | `MAG-GEN-AGENT-LIMIT` | Retry, or prepare continuation after potentially mutating work |
| Repeated tool batches | `MAG-GEN-REPEATED-ACTIONS` | Retry, or prepare continuation after potentially mutating work |
| Unknown | `MAG-GEN-UNKNOWN` | Retry or choose another model |

A locally oversized turn is rejected during preparation with
`StorageErrorKind::ContextTooLarge` and `MAG-CONTEXT-TOO-LARGE` composer copy.
It does not require creating a failed assistant record or calling the provider.
Account failures use `MAG-ACCOUNT-*`; storage setup/read/write use
`MAG-STORAGE-INIT`, `MAG-STORAGE-LOAD`, and `MAG-STORAGE-WRITE`.

## Retry, regeneration, and continuation

- **Retry** creates a fresh assistant record for the failed user turn, retaining
  the failed attempt. Context is selected before that failed assistant so its
  partial output is not replayed. A generation override can select another model.
- **Regenerate** prepares replacement content in the addressed assistant record.
  It is a separate workflow from retrying a failure.
- **Prepare continuation** fills the composer with a draft asking the agent to
  inspect the current state and continue completed work. The user reviews and
  sends it. It neither resumes saved tool protocol state nor grants new approvals.

UI recovery uses recorded `create_file`, `apply_patch`, and `run_command`
activity as evidence that work may have changed the workspace. The response
retry action routes failed attempts with such activity to a prepared
continuation; the agent-limit/repeated-action cards do likewise. Other failure
cards have category-specific recovery, so callers must still account for side
effects when adding retry paths. There is no universal automatic replay or
rollback of agent tools. Each new run starts with fresh approval state.

## Async lifecycle and persistence

Keep lifecycle-bound GPUI tasks in the owning entity. Increment a generation
for replaceable work, then ignore completions from an older conversation,
project, search query, or file load. Run blocking I/O through background workers;
only update live GPUI entities on the application thread.

Preserve user input and prior usable content during retry. Persist turn creation
before provider work and terminal responses after it; do not connect SQLite
writes to text-delta events. Agent activity records have their own persistence
path. Detached tasks are reserved for deliberate longer-lived work and need
explicit cancellation/error handling.

Settings reload applies only after successful parsing. Failed saves keep the
in-memory preference available for recovery. TOML writes use a flushed
sibling file and rename. Reset backs up the old file first, then uses the normal
save path; it does not bypass malformed TOML. See
[storage](../crates/storage/README.md#settings-behavior).

## Diagnostics and verification

Diagnostics initialize before fallible application setup. `RUST_LOG` can raise
verbosity; files rotate daily with seven retained in the local `magenta/logs`
directory. Failure to create that sink falls back to stderr. No log upload is
implemented.

Storage UI logs use stable operation/error codes rather than raw SQLite source
text, which can contain stored content. Provider and other technical logs can
retain source chains, so do not assume log files are redacted just because the
UI card is. Never add credentials, full prompts, clipboard contents, or provider
bodies as routine diagnostic fields.

Changes to failure behavior should check the domain mapping, adapter
classification, persistence round-trip, and UI action that owns recovery.
Existing tests cover safe failure serialization, HTTP/agent error categories,
retry persistence, stale async completions, and contextual UI states. Use the
[crate guides](../README.md#documentation-and-crate-ownership) for focused test
commands and [development](development.md) for the full verification workflow.
