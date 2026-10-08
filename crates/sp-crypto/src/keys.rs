//! The two P-256 keys of a device: one signs (DPoP proofs, commits), one
//! receives HPKE-sealed messages (the welcome, key envelopes).

use crate::{CryptoError, Result, b64};
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate;
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sp_core::canonical::canonical_json;

/// A P-256 public key as a JWK (`{kty, crv, x, y}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicJwk {
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub y: String,
}

impl PublicJwk {
    fn of(key: &PublicKey) -> Result<Self> {
        let point = key.to_sec1_point(false);
        let (Some(x), Some(y)) = (point.x(), point.y()) else {
            return Err(CryptoError::InvalidKey);
        };
        Ok(Self {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: b64::encode(x),
            y: b64::encode(y),
        })
    }

    /// Canonical JSON, as hashed by the confirm code and the thumbprint.
    pub fn canonical(&self) -> String {
        canonical_json(&json!({ "kty": self.kty, "crv": self.crv, "x": self.x, "y": self.y }))
    }

    /// RFC 7638 SHA-256 thumbprint, base64url: the DPoP `jkt`.
    pub fn thumbprint(&self) -> String {
        b64::encode(Sha256::digest(self.canonical().as_bytes()))
    }
}

fn secret_from_b64(text: &str) -> Result<SecretKey> {
    let bytes = b64::decode(text).ok_or(CryptoError::InvalidKey)?;
    SecretKey::from_slice(&bytes).map_err(|_| CryptoError::InvalidKey)
}

/// The signing key (ES256). Its thumbprint binds the device's tokens.
#[derive(Clone)]
pub struct SignKey(SigningKey);

impl SignKey {
    pub fn generate() -> Result<Self> {
        SigningKey::try_generate()
            .map(Self)
            .map_err(|_| CryptoError::Random)
    }

    pub fn from_b64(text: &str) -> Result<Self> {
        Ok(Self(SigningKey::from(secret_from_b64(text)?)))
    }

    /// The private scalar, base64url. Store it only in private files.
    pub fn to_b64(&self) -> String {
        b64::encode(self.0.to_bytes())
    }

    pub fn public_jwk(&self) -> Result<PublicJwk> {
        PublicJwk::of(&PublicKey::from(self.0.verifying_key()))
    }

    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.0
    }
}

/// The HPKE recipient key (DHKEM P-256).
#[derive(Clone)]
pub struct EncKey(SecretKey);

impl EncKey {
    pub fn generate() -> Result<Self> {
        SecretKey::try_generate()
            .map(Self)
            .map_err(|_| CryptoError::Random)
    }

    pub fn from_b64(text: &str) -> Result<Self> {
        secret_from_b64(text).map(Self)
    }

    pub fn to_b64(&self) -> String {
        b64::encode(self.0.to_bytes())
    }

    pub fn public_jwk(&self) -> Result<PublicJwk> {
        PublicJwk::of(&self.0.public_key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7638_thumbprint() {
        // RFC 7638 §3.1 uses RSA; this P-256 vector comes from RFC 9449 §4.1/§6.1.
        let jwk = PublicJwk {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs".into(),
            y: "9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA".into(),
        };
        assert_eq!(
            jwk.thumbprint(),
            "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I"
        );
    }

    #[test]
    fn keys_survive_a_round_trip() -> Result<()> {
        let sign = SignKey::generate()?;
        let restored = SignKey::from_b64(&sign.to_b64())?;
        assert_eq!(sign.public_jwk()?, restored.public_jwk()?);

        let enc = EncKey::generate()?;
        let jwk = EncKey::from_b64(&enc.to_b64())?.public_jwk()?;
        assert_eq!(jwk, enc.public_jwk()?);
        assert_eq!(b64::decode(&jwk.x).map(|x| x.len()), Some(32));
        assert_ne!(sign.public_jwk()?, jwk);
        Ok(())
    }

    #[test]
    fn rejects_garbage_keys() {
        assert!(SignKey::from_b64("not-a-key").is_err());
        assert!(EncKey::from_b64(&b64::encode([0u8; 32])).is_err());
    }
}
