//! PKCE (S256) for the CLI login.

use crate::{Result, b64, random_bytes};
use sha2::{Digest, Sha256};

pub struct Pkce {
    /// 43 base64url characters; sent only when redeeming.
    pub verifier: String,
    /// base64url(SHA-256(verifier)); sent when the request is opened.
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Result<Self> {
        let verifier = b64::encode(random_bytes::<32>()?);
        Ok(Self {
            challenge: challenge(&verifier),
            verifier,
        })
    }
}

pub fn challenge(verifier: &str) -> String {
    b64::encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_has_the_length_the_server_accepts() -> Result<()> {
        let pkce = Pkce::generate()?;
        assert_eq!(pkce.verifier.len(), 43);
        assert_eq!(pkce.challenge, challenge(&pkce.verifier));
        Ok(())
    }
}
