//! Device keys, DPoP proofs, PKCE and the login confirm code. Byte-compatible
//! with `@skillpouch/crypto` and the API.

pub mod b64;
pub mod confirm;
pub mod dpop;
pub mod keys;
pub mod pkce;

pub use keys::{EncKey, PublicJwk, SignKey};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("the system random number generator failed")]
    Random,
    #[error("invalid key")]
    InvalidKey,
}

pub type Result<T> = std::result::Result<T, CryptoError>;

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| CryptoError::Random)?;
    Ok(bytes)
}
