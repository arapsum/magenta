//! Workspace filesystem and sandboxed-command adapters for Magenta.
//!
//! [`LocalWorkspace`] provides root-confined browsing and preview/commit file
//! operations. [`BubblewrapCommandRunner`] probes Linux command isolation before
//! use. These adapters enforce access constraints; application callers own user
//! approval before committing mutations or executing commands.

mod agentfs;
mod command;
mod indexing;
mod operations;
mod patch;
mod path;
mod repository;

pub use agentfs::AgentFsWorkspace;
pub use command::BubblewrapCommandRunner;
pub use indexing::WorkspaceCodeIndexer;
pub use operations::LocalWorkspace;
pub use repository::LocalRepository;
