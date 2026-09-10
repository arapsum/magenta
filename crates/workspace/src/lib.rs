//! Workspace filesystem and sandboxed-command adapters for Magenta.
//!
//! [`LocalWorkspace`] provides root-confined browsing and preview/commit file
//! operations. [`BubblewrapCommandRunner`] probes Linux command isolation before
//! use. These adapters enforce access constraints; application callers own user
//! approval before committing mutations or executing commands.

mod command;
mod operations;
mod patch;
mod path;

pub use command::BubblewrapCommandRunner;
pub use operations::LocalWorkspace;
