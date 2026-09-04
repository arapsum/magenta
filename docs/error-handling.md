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

Diagnostics initialize before fallible application setup. Production defaults
record Magenta information and warnings from GPUI; `RUST_LOG` can increase
development verbosity. Logs rotate daily with seven files retained in the
platform-local Magenta data directory. If that directory is unavailable,
Magenta continues with stderr diagnostics and warns the user after the main
window opens.

No diagnostics are transmitted remotely. Adding crash upload or telemetry
requires a separate privacy and consent decision.
