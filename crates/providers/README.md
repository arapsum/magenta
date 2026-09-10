# magenta-providers

Provider adapters implementing the contracts in `magenta-core`. This crate
owns HTTP, authentication, wire serialization, and stream decoding. It does not
render UI, persist conversations, or execute workspace tools.
[Workspace overview](../../README.md).

## Public API

[src/lib.rs](src/lib.rs) exports two providers:

- `OpenAiProvider::new()` implements `ChatProvider`, `AgentProvider`,
  `ModelCatalog`, and `ProviderAuthenticator`. Clones share its authentication
  and HTTP state, allowing desktop composition to inject the same account into
  chat, agent, and model-discovery workflows.
- `DemoProvider::new(initial_delay, chunk_delay)` implements `ChatProvider` with
  deterministic local responses. Zero durations are useful in tests. It does
  not implement account login, model discovery, or workspace-agent tools.

The current OpenAI adapter uses ChatGPT OAuth and the Codex backend endpoint
configured in [openai/mod.rs](src/openai/mod.rs). It is not an API-key-configured
adapter for the public OpenAI API. Other provider logos in the application are
assets, not implemented adapters.

## Source map

| Module | Responsibility |
| --- | --- |
| [openai/mod.rs](src/openai/mod.rs) | Model catalog, chat streaming, request headers, HTTP error classification |
| [openai/auth/mod.rs](src/openai/auth/mod.rs) | Browser login, loopback callback, credential restore, refresh and sign-out |
| [openai/auth/claims.rs](src/openai/auth/claims.rs) | Account metadata extraction from authentication claims |
| [openai/wire.rs](src/openai/wire.rs) | Request/response payloads, model metadata, attachments and continuation serialization |
| [openai/sse.rs](src/openai/sse.rs) | Server-sent-event framing and decoding |
| [openai/agent.rs](src/openai/agent.rs) | Tool-call streams and provider-specific start/resume handling |
| [openai/http.rs](src/openai/http.rs) | HTTP construction, sending and bounded body reads |
| [demo.rs](src/demo.rs), [contract.rs](src/contract.rs) | Deterministic provider and test contract helpers |

## Authentication and streaming

`begin_login` returns an `AuthorizationSession`: the UI opens its browser URL
and waits for its completion future. The adapter uses PKCE and validates the
callback state. The current loopback callback uses port 1455 and a five-minute
timeout. Credentials use the OS keyring under service `dev.magenta.desktop`
and account `openai-codex`, not the settings TOML or history database.

The adapter restores accounts, refreshes expiring credentials, and retries an
unauthorized responses request once after forced refresh. Wire and transport
failures map to `ProviderErrorKind`; only allowlisted HTTP diagnostics may be
persisted as `MessageFailure`. Raw response bodies stay outside user-facing
failure payloads.

Chat streams emit start, text deltas, and one terminal outcome, or a typed
failure. Agent streams additionally return tool calls and opaque continuation
payloads. The application executes the tools and supplies their results on
`resume`; the adapter does not grant approvals or access the workspace itself.
Images are serialized from validated local attachments for supported requests.

## Verify and extend

From the repository root:

```bash
cargo test --locked -p magenta-providers
cargo doc --locked -p magenta-providers --no-deps
```

Tests cover deterministic stream contracts, SSE parsing, authentication with
test doubles, wire payloads, and failure classification. They do not establish
live account access or remote service availability. Real sign-in, discovery,
and streaming need a manual check with a connected account.

A new provider should implement the needed core ports, map its failures and
completion metadata, and add stream/wire tests. Wire it in the desktop crate;
do not introduce provider-specific JSON into UI or application workflows.
See [error handling](../../docs/error-handling.md).
