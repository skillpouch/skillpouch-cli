//! HTTP client for the SkillPouch API: the `{ ok, data }` envelope, DPoP
//! proofs with server nonces, and typed calls for login and the account.

mod client;
mod error;
pub mod types;

pub use client::Client;
pub use error::{ApiError, Result};
