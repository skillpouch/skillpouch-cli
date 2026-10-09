//! Device keys, DPoP proofs, PKCE, the login confirm code, and reading the
//! account's encrypted metadata (roster, key envelopes, pouch names). Byte-compatible
//! with `@skillpouch/crypto` and the API.

pub mod b64;
pub mod confirm;
pub mod dpop;
mod hpke;
pub mod keys;
pub mod pkce;
pub mod pouch;
pub mod sealed;
pub mod vault;

pub use keys::{EncKey, PublicJwk, SignKey};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("the system random number generator failed")]
    Random,
    #[error("invalid key")]
    InvalidKey,
    #[error("unsupported format: {0}")]
    Format(&'static str),
    #[error("ciphertext failed authentication")]
    Aead,
    #[error("signature does not verify: {0}")]
    Signature(String),
    #[error("the account roster can't be trusted: {0}")]
    Roster(String),
}

pub type Result<T> = std::result::Result<T, CryptoError>;

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| CryptoError::Random)?;
    Ok(bytes)
}
