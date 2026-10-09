//! The two P-256 keys of a device: one signs (DPoP proofs, commits), one
//! receives HPKE-sealed messages (the welcome, key envelopes).

use crate::{CryptoError, Result, b64};
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
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

    /// The uncompressed SEC1 point (`0x04 ‖ x ‖ y`).
    pub fn point(&self) -> Result<Vec<u8>> {
        let x = b64::decode(&self.x).filter(|x| x.len() == 32);
        let y = b64::decode(&self.y).filter(|y| y.len() == 32);
        let (Some(x), Some(y)) = (x, y) else {
            return Err(CryptoError::InvalidKey);
        };
        Ok([&[4u8][..], &x, &y].concat())
    }

    /// Checks a raw (64-byte, base64url) ES256 signature over `context ‖ message`,
    /// as `@skillpouch/crypto` signs roster statements and key envelopes.
    pub fn verifies(&self, signature: &str, context: &str, message: &[u8]) -> bool {
        let Ok(point) = self.point() else {
            return false;
        };
        let Ok(key) = VerifyingKey::from_sec1_bytes(&point) else {
            return false;
        };
        let Some(signature) = b64::decode(signature)
            .filter(|bytes| bytes.len() == 64)
            .and_then(|bytes| Signature::from_slice(&bytes).ok())
        else {
            return false;
        };
        key.verify(&[context.as_bytes(), message].concat(), &signature)
            .is_ok()
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

    /// A raw ES256 signature (base64url) over `context ‖ message`: how
    /// members sign revisions, as `PublicJwk::verifies` checks them.
    pub fn sign(&self, context: &str, message: &[u8]) -> String {
        let signature: Signature = self.0.sign(&[context.as_bytes(), message].concat());
        b64::encode(signature.to_bytes())
    }

    /// A detached compact JWS (`header..signature`) over `payload`, as the
    /// `Skillpouch-Signature` header of a commit.
    pub fn detached_jws(&self, payload: &[u8]) -> String {
        let header = b64::encode(br#"{"alg":"ES256"}"#);
        let signing_input = format!("{header}.{}", b64::encode(payload));
        let signature: Signature = self.0.sign(signing_input.as_bytes());
        format!("{header}..{}", b64::encode(signature.to_bytes()))
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

    pub(crate) fn scalar(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
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
    fn signatures_verify_under_their_context() -> Result<()> {
        let key = SignKey::generate()?;
        let jwk = key.public_jwk()?;
        let signature = key.sign("skillpouch/v1/revision-sig", b"message");
        assert!(jwk.verifies(&signature, "skillpouch/v1/revision-sig", b"message"));
        assert!(!jwk.verifies(&signature, "skillpouch/v1/roster", b"message"));
        let jws = key.detached_jws(b"digest");
        let parts: Vec<&str> = jws.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1], "");
        Ok(())
    }

    #[test]
    fn rejects_garbage_keys() {
        assert!(SignKey::from_b64("not-a-key").is_err());
        assert!(EncKey::from_b64(&b64::encode([0u8; 32])).is_err());
    }
}
