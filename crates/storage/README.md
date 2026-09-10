# magenta-storage

Local persistence adapters for Magenta: SQLite conversations and projects,
managed image attachments, and TOML settings. Core traits are its public
boundary; database and filesystem work runs on `smol::unblock` workers.
[Workspace overview](../../README.md).

## Public API and initialization

- `SqliteConversationStore::new(path)` implements `ConversationStore` and
  `ProjectStore`. Call and await `ConversationStore::initialize` before using
  it. Initialization creates directories/schema, migrates existing data,
  recovers interrupted runs, and reconciles managed attachments.
- `TomlSettingsStore::new(path)` implements `SettingsStore`. It needs no separate
  initialization; loading a missing file returns `AppSettings::default()`.

Constructors accept paths rather than choosing user directories. The desktop
crate supplies the production locations listed in the
[project README](../../README.md#local-data-and-settings).

## SQLite and turn lifecycle

[src/lib.rs](src/lib.rs) implements the ports. Connections enable foreign keys,
a five-second busy timeout, and WAL journaling during initialization. Immediate
transactions protect turn preparation, and a partial unique index permits only
one streaming assistant per conversation.

[turns.rs](src/turns.rs) handles these distinct operations:

- `begin_turn`: import user input, select budgeted context, and commit a user
  message plus assistant placeholder before provider execution.
- `begin_regeneration`: reset the addressed assistant for replacement output.
- `begin_retry`: retain the failed assistant and append a new assistant attempt
  using context strictly before the failed response.
- `finalize` in `lib.rs`: save terminal text, outcome, failure metadata and agent
  run status for the addressed streaming response.

[records.rs](src/records.rs) maps rows to domain values and reads up to
50-message pages, including pages around a search match. Provider context
queries completed messages independently of the visible page, then uses core's
whole-turn budget selector. Failed/stopped partial output is excluded. Context
queries currently materialize eligible history before selection; the rendered
page limit is not a bound on that temporary worker allocation.

SQLite FTS5 indexes titles and message content. [search.rs](src/search.rs) builds
search results with snippets and highlights; triggers keep indexes in sync
with normal writes. Renaming preserves pin/recency state. Conditional rename
prevents a generated title from replacing a concurrent manual rename.

## Schema and attachments

[schema.sql](src/schema.sql) defines schema version **7**.
[migrations](src/migrations/mod.rs) upgrade versions 1–6 transactionally:
managed-attachment metadata (v2), agent runs/activity (v3), projects (v4), FTS
search (v5), omitted-context counts (v6), and structured response failures (v7).
Unknown schema versions are rejected. Update both fresh schema and migrations
when changing persisted fields.

Initialization marks unfinished messages and agent runs stopped. Text deltas
are not persisted continuously, so a crash can lose text since turn creation.

[attachments.rs](src/attachments.rs) validates PNG, JPEG, WebP, and non-animated
GIF images, with limits of four images per turn and 10 MiB per image. Imported
files get opaque names in the database's sibling `attachments/` directory.
Failed imports/turns clean up their managed copies; reconciliation handles
orphaned files. Deletion removes managed copies without deleting original
sources. Path encoding and managed-file checks also live in
[records.rs](src/records.rs) and [managed_attachments.rs](src/managed_attachments.rs).

## Settings behavior

[settings.rs](src/settings.rs) preserves TOML comments and unknown keys. Missing
or invalid individual values fall back to supported defaults; malformed TOML
returns an error. Saves are serialized within a shared store, written to a
temporary sibling, flushed, and renamed. Reset first backs up an existing file,
then saves defaults through the same parser/writer; malformed TOML must be
repaired before reset can finish. Provider credentials never enter this adapter.

## Verify

From the repository root:

```bash
cargo test --locked -p magenta-storage
cargo doc --locked -p magenta-storage --no-deps
```

[Persistence tests](tests/persistence.rs) cover schema/history behavior and retry;
[context and paging tests](tests/context_and_paging.rs) cover bounds and
context selection; [attachment tests](tests/attachments.rs) cover ownership and
cleanup. Settings have unit tests alongside their implementation. Use temporary
directories in tests, not the user's database. See
[error handling](../../docs/error-handling.md) for save and migration failures.
