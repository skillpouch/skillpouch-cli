//! Keeping a pouch and this computer in sync.
//!
//! The pouch lives in plain files under `~/.skillpouch/pouches/<id>/`, one
//! folder per item (`skills/pdf/SKILL.md`, `mcp-servers/github/server.json`).
//! The engine compares that folder, the server and what both looked like
//! after the last sync, then pushes local changes and pulls remote ones.
//! Projection links the items into each assistant's own folders and writes
//! MCP servers, hooks and plugins into their config files.

pub mod adopt;
pub mod approvals;
mod command;
pub mod engine;
mod error;
pub mod hooks;
pub mod item;
mod jsonc;
pub mod mcp;
pub mod noticed;
pub mod plugins;
pub mod project;
pub mod remote;
pub mod state;
pub mod status;

pub use error::{Result, SyncError};
