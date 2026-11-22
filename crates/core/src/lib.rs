//! Bifold core: filesystem model, dual-pane session state, transfer jobs.
//!
//! The crate is UI-agnostic on purpose: the TUI (`bifold-tui`) and the desktop
//! GUI (`bifold-gui`) render the same [`Session`] and drive the same
//! [`fsops`] functions, so behaviour never drifts between the two front ends.

pub mod complete;
pub mod config;
pub mod entry;
pub mod error;
pub mod filter;
pub mod format;
pub mod fsops;
pub mod help;
pub mod job;
pub mod keys;
pub mod pane;
pub mod preview;
pub mod runcmd;
pub mod session;
pub mod sort;
pub mod style;
pub mod theme;
mod theme_builtin;

pub use entry::{Entry, EntryKind};
pub use error::{Error, Result};
pub use pane::Pane;
pub use session::{LEFT, RIGHT, Session};

/// Product name, used by both front ends.
pub const APP_NAME: &str = "Bifold";
/// Crate version, from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
