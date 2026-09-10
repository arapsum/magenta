# magenta-application

Headless workflows connecting Magenta's domain ports. The only internal runtime
dependency is `magenta-core`; persistence tests additionally use
`magenta-storage`. No GPUI entities, SQLite connections, or provider wire types
belong here. [Workspace overview](../../README.md).

## Public entry points

Exports are collected in [src/lib.rs](src/lib.rs).

| Workflow | Responsibility | Source |
| --- | --- | --- |
| `SendMessage` | Validate input, persist a turn, return its generation stream; generate a title without overwriting a later manual rename | [send_message.rs](src/send_message.rs) |
| `RegenerateMessage::execute` | Prepare replacement output for an addressed assistant message | [regenerate_message.rs](src/regenerate_message.rs) |
| `RegenerateMessage::retry` | Append an assistant attempt while retaining failed output; optionally use a different generation configuration | [regenerate_message.rs](src/regenerate_message.rs) |
| `ConversationHistory` | Initialize, page, search, finalize, rename, pin, and delete through the store port | [history.rs](src/history.rs) |
| `ProjectCatalog` | Canonicalize/register projects, update recency, forget registrations, browse files | [projects/mod.rs](src/projects/mod.rs) |
| `RunWorkspaceAgent` | Prepare a persisted agent turn or retry, then orchestrate provider steps, tools, and approvals | [agent/mod.rs](src/agent/mod.rs) |

Construct workflows with `Arc<dyn ...>` implementations of core ports. Await
`execute`/`retry` before consuming the returned `PendingGeneration`,
`PendingRegeneration`, `PendingRetry`, or `PendingAgentGeneration` stream.
The pending value includes the persisted message identity and context report.
The caller owns stream consumption, cancellation, and terminal-message saving.

Preparation must finish before provider invocation. If validation, attachment
import, or context selection fails, no provider request should start. Retry
selects context before the failed assistant and retains that original record;
it is different from replacement-style regeneration.

## Agent loop and approvals

[agent/stream.rs](src/agent/stream.rs) consumes a provider step, gathers tool
calls, checks the loop guard, executes the batch, and resumes the provider with
tool outputs. Calls within a batch execute sequentially. The current guard
allows 64 tool rounds and 256 tool calls, and rejects a third consecutive
identical batch. Fingerprints compare names and canonicalized JSON arguments,
not provider-generated call IDs. Limit and repetition failures have typed
provider error categories.

[agent/tools.rs](src/agent/tools.rs) defines `list_files`, `search_text`,
`read_file`, `create_file`, and `apply_patch`. `run_command` is advertised only
when a command runner was injected. Command execution is coordinated in
[agent/commands.rs](src/agent/commands.rs).

File operations prepare a preview before commit. The stream emits proposed
changes and approval requests; `AgentApprovalController::decide` answers the
matching request ID. `ApproveWorkspaceEditsForRun` covers eligible, unprotected
creates and patches for that stream only. Protected reads and commands retain
individual approval. Permission state is not saved or reused for a retry.

Tool calls, approval requests, and results are recorded through the store, while
change and command-output events drive the UI. Expected tool failures become
tool results the provider can respond to. Orchestration/provider failures end
the stream. File and sandbox enforcement live in `magenta-workspace`; the UI
is responsible for presenting previews and collecting user decisions.

Provider `resume` is an in-run protocol continuation. A user's **Prepare
continuation** action is a separate UI workflow that creates a draft after
partial work; it is not a persisted tool replay or automatic loop resumption.

## Verify

From the repository root:

```bash
cargo test --locked -p magenta-application
cargo test --locked -p magenta-application --test persisted_workflows
cargo doc --locked -p magenta-application --no-deps
```

[Integration tests](tests/persisted_workflows.rs) exercise workflows against
SQLite and fake providers. Agent unit tests cover approvals, command/tool
behavior, batch accounting, and repeated-call detection. Changes to workflow
failure categories should also update [error handling](../../docs/error-handling.md).
