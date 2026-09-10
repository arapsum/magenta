# magenta-workspace

Filesystem and process adapters for a selected workspace. This crate implements
core ports without depending on the UI, provider, or storage crates. Approval
orchestration belongs to `magenta-application`; callers of these adapters must
enforce their own authorization before committing mutations or running commands.
[Workspace overview](../../README.md).

## Public API

- `LocalWorkspace` implements `WorkspaceAccess` for prepared reads/mutations
  and `WorkspaceBrowser` for project canonicalization, direct-child directory
  listings, and bounded UTF-8 document reads.
- `BubblewrapCommandRunner::new()` discovers `bwrap` and performs an isolation
  probe. On success it implements `WorkspaceCommandRunner`; on failure desktop
  composition omits the runner and exposes files-only Agent mode.

Exports are in [src/lib.rs](src/lib.rs). File work runs on blocking workers;
commands stream start, stdout/stderr chunks, and terminal results.

## File access and previews

[path.rs](src/path.rs) canonicalizes roots and validates relative paths. Absolute
paths, parent traversal, `.git` components, and resolved paths outside the root
are rejected. Creating a missing path also validates its nearest existing
parent. Use `.` for the root in agent file/command tools.

[operations.rs](src/operations.rs) implements listing, literal search, reads,
new-file creation, and patch preparation. Agent reads of protected files require
the caller's explicit `allow_protected` decision. Protected names include `.env`
variants, package credential files, SSH private-key names, and common key/cert
extensions. Protected creates and patches are rejected. The human-facing
`WorkspaceBrowser` is a separate read-only path, without the agent approval
flow; it still enforces root confinement, UTF-8, and the document size bound.

`prepare` returns a `WorkspacePreview` and, when appropriate, a mutation with
replacement bytes and an expected digest. `commit` revalidates the destination
and the prior content digest before writing. New files must not already exist.
[patch.rs](src/patch.rs) applies the supported unified patch format to one file;
it is not a shell invocation or a repository-wide Git patch operation.

Current file limits are defined in `path.rs` and `operations.rs`:

| Operation | Limit |
| --- | --- |
| File reads, browser documents, new/replacement content | 1 MiB |
| Agent read response | 256 KiB |
| Input patch | 256 KiB |
| Agent file listing | 500 entries |
| Text search | 100 matches or 128 KiB of output |

The 500-entry cap belongs to the agent listing operation; it is not a cap on the
project explorer's direct-child listings. Listings/search use `ignore` walking
and their configured filters; browse and agent policies are deliberately
implemented separately.

## Command sandbox

[command/mod.rs](src/command/mod.rs) passes an executable and argument vector
directly to Bubblewrap. It does not implicitly interpolate a shell command.
The sandbox binds the selected workspace writable at `/workspace`, exposes
read-only system/discovered toolchain mounts, masks protected workspace files
and Git metadata, clears the inherited environment, and creates a temporary
home. It has no network and no stdin. Host home access is limited to selected
toolchain/cache mounts rather than mounting the entire home directory.

Command arguments are limited to 64 entries and 32 KiB combined. A zero timeout
uses 120 seconds; the maximum is 600 seconds. Captured output keeps bounded
head/tail content (32 KiB per stdout/stderr capture) and UI streaming is capped
at 128 KiB per output reader. Results distinguish exit, timeout and cancellation;
validation or sandbox setup can fail before the command starts. Dropping the
consumer stream triggers process cancellation through the output receiver.

Rust/Node/pnpm discovery is performed when constructing the runner. pnpm uses
offline mode, a disposable writable store index, and read-only cached package
content when available. A host-installed executable or cached dependency is not
guaranteed to be discoverable in the sandbox. There is no unrestricted fallback
or network-enabled command policy.

## Verify

From the repository root:

```bash
cargo test --locked -p magenta-workspace
cargo doc --locked -p magenta-workspace --no-deps
```

Tests cover file operations, patch behavior and sandbox preparation.
[Command integration tests](src/command/tests.rs) require Linux, Bubblewrap and
working namespaces; they return early if the isolation probe fails. Exercise
real command execution on a supported host to verify those paths. Use
disposable workspaces for mutation tests.
