# Error handling

Magenta uses typed errors for technical context and separate presentation data
for user-facing recovery. The shared application result type is:

```rust
pub type Result<T> = std::result::Result<T, MagentaError>;
```

Add a `MagentaError` variant when a new subsystem needs a distinct recovery
decision. Preserve the original error with `#[source]` or an unambiguous
`#[from]` conversion. Do not add a string-only or catch-all variant merely to
make `?` compile.

## Presentation policy

| Failure | Presentation | Clearing event |
| --- | --- | --- |
| Invalid input | Inline beside the control | Relevant input changes or validation succeeds |
| Failed user action | Persistent notification with Retry or Dismiss | Retry succeeds or the user dismisses it |
| Failed view/data load | Inline state that preserves prior usable content | Reload succeeds or navigation replaces the view |
| Recoverable startup issue | Safe fallback and persistent warning | The user dismisses it or a later startup succeeds |
| Failure before a window exists | Local diagnostics and a failing process status | A later launch succeeds |
| Invariant violation | Panic diagnostics followed by normal panic handling | Not recoverable in-process |

Raw error sources, local paths, credentials, prompts, clipboard contents, and
other user-generated content must not appear in UI messages. Use
`MagentaError::presentation` for stable codes and privacy-safe copy. Log the
typed error with its operation name and source chain for developers.

## Async operations

An entity that owns asynchronous work also owns its state and task:

```rust
enum LoadState<T> {
    Idle,
    Loading { generation: u64 },
    Ready(T),
    Empty,
    Failed { error: MagentaError, retryable: bool },
}
```

- Keep lifecycle-bound GPUI `Task` values in the owning entity so dropping or
  replacing the owner cancels the work.
- Increment a generation before starting replaceable work and ignore stale
  completions.
- Run blocking I/O and expensive parsing on the background executor, then
  update live entities on the application thread.
- Preserve user input and previous usable content while retrying.
- A retry repeats the original operation with the same validated input.
- Detached tasks are reserved for deliberate application-lifetime work and
  must record their failures.

## Diagnostics

Conversation storage failures use `StorageError` in `core`, with operation
errors propagated through the application layer. `MAG-STORAGE-INIT`,
`MAG-STORAGE-LOAD`, and `MAG-STORAGE-WRITE` provide distinct UI recovery paths.
Initialization failure disables sending until retry succeeds. A failed read
keeps the previous selection; a failed response save keeps the visible text
and blocks navigation until Retry succeeds. Pinning changes appear only after
their write succeeds.

Storage notifications expose no SQL, paths, or message contents. Storage UI
logs use stable operation/error codes rather than raw SQLite diagnostics,
which can contain stored text. Do not connect persistence to streamed text
delta events: persist turn creation and terminal responses only.

Diagnostics initialize before fallible application setup. Production defaults
record Magenta information and warnings from GPUI; `RUST_LOG` can increase
development verbosity. Logs rotate daily with seven files retained in the
platform-local Magenta data directory. If that directory is unavailable,
Magenta continues with stderr diagnostics and warns the user after the main
window opens.

No diagnostics are transmitted remotely. Adding crash upload or telemetry
requires a separate privacy and consent decision.

## Settings persistence

Settings are preferences, not credentials. The TOML file stores appearance and
typography values; provider credentials remain in the operating system keyring.

- A missing settings file loads the versioned defaults.
- A malformed or unreadable file must not replace the currently usable settings;
  record the typed settings error and offer a retry or an edit through the
  settings window.
- Reload applies a file only after it has been read and parsed successfully.
- Saves are written to a temporary sibling, flushed, and renamed into place so
  an interrupted write does not leave a partially written settings file.
- Restoring defaults creates a timestamped backup before replacing the active
  file. If the backup cannot be created, the reset must not proceed.

Settings errors should use the same presentation rules as other local I/O:
show a concise recovery-oriented status in the settings window and keep the
technical source chain in diagnostics rather than exposing filesystem details
or credentials in normal UI copy.
