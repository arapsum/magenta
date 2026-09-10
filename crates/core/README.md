# magenta-core

Provider-independent domain values and ports shared by the rest of Magenta.
This crate has no GPUI, HTTP, database, filesystem adapter, or process runner
dependency. [Workspace overview](../../README.md).

## Public API

All public types are re-exported from [src/lib.rs](src/lib.rs).

| Area | Main types and traits | Source |
| --- | --- | --- |
| Conversations | `Conversation`, `Message`, attachments, roles, statuses, typed IDs | [message.rs](src/message.rs), [conversation.rs](src/conversation.rs), [identifiers.rs](src/identifiers.rs) |
| Generation | `ChatProvider`, `GenerationRequest`, `GenerationEvent`, `GenerationConfig`, effort, limits, outcomes and token usage | [generation.rs](src/generation.rs) |
| Accounts and models | `ProviderAuthenticator`, `AuthorizationSession`, `ModelCatalog`, `ModelDescriptor` | [auth.rs](src/auth.rs), [models.rs](src/models.rs) |
| Persistence | `ConversationStore`, pages, summaries, search results, `PreparedTurn`, `StorageError` | [storage.rs](src/storage.rs) |
| Settings | `SettingsStore`, `AppSettings`, typography and appearance choices | [settings.rs](src/settings.rs) |
| Projects and browsing | `ProjectStore`, `WorkspaceBrowser`, `Project`, `WorkspaceDocument` | [project.rs](src/project.rs) |
| Agent execution | `AgentProvider`, request/continuation types, tools, approvals and run events | [agent/mod.rs](src/agent/mod.rs) |
| Workspace operations | `WorkspaceAccess`, `WorkspacePreview`, `WorkspaceMutation`, `WorkspaceCommandRunner` | [workspace.rs](src/workspace.rs), [command.rs](src/command.rs) |
| Failure reporting | `ProviderError`, safe diagnostics, `MessageFailure` | [error.rs](src/error.rs), [message.rs](src/message.rs) |

Ports return boxed futures or streams so callers are not tied to a concrete
adapter. Storage owns durable IDs and message sequences. Generation streams
carry start, text-delta, and completion events; failures use `ProviderError`.
Agent continuations keep provider-specific state in opaque bytes.

## Context selection

[context.rs](src/context.rs) implements `select_context`, `estimate_text_tokens`,
and `estimate_agent_overhead`. Selection keeps the newest contiguous whole user
turns that fit after reserving maximum output tokens and a 5% context margin.
It reports estimated input, the available input budget, and omitted messages.
If the newest user turn cannot fit, selection returns `ContextTooLarge`.

Text is estimated from UTF-8 byte length, with additional message/request
framing and a fixed image allowance. This is deliberately approximate. The
caller supplies generation limits and tool/instruction overhead; core neither
fetches model metadata nor summarizes omitted history.

## Contract rules

- Keep external wire formats and implementation types inside adapters. A new
  backend implements these ports rather than changing UI code to call it.
- Atomic turn preparation and addressed-message finalization belong to
  `ConversationStore`; the store rejects concurrent streaming turns.
- `MessageFailure` persists categories, reference codes, provider IDs, and
  allowlisted details. It must not serialize raw provider error sources.
- Settings are preferences; credential storage belongs behind the provider
  authentication port. Persisted format changes need compatible adapter changes.

## Verify

From the repository root:

```bash
cargo test --locked -p magenta-core
cargo doc --locked -p magenta-core --no-deps
```

Unit tests cover context selection, configuration values, domain types, and
safe failure serialization. See [development](../../docs/development.md) for
workspace checks and [error handling](../../docs/error-handling.md) for policy.
