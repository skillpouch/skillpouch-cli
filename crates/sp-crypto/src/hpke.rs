//! HPKE (RFC 9180) base mode with DHKEM(P-256, HKDF-SHA256), HKDF-SHA256 and
//! ChaCha20-Poly1305: how the welcome and key envelopes reach a device.

use crate::keys::{EncKey, PublicJwk};
use crate::{CryptoError, Result};
use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::DhP256HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};

type Suite = DhP256HkdfSha256;

/// Seals `plaintext` to `recipient`: `(enc, ciphertext)`.
pub fn seal(
    recipient: &PublicJwk,
    info: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>)> {
    let public = <Suite as Kem>::PublicKey::from_bytes(&recipient.point()?)
        .map_err(|_| CryptoError::InvalidKey)?;
    let (enc, ciphertext) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, Suite>(
        &OpModeS::Base,
        &public,
        info,
        plaintext,
        aad,
    )
    .map_err(|_| CryptoError::Aead)?;
    Ok((enc.to_bytes().to_vec(), ciphertext))
}

pub fn open(
    key: &EncKey,
    enc: &[u8],
    ciphertext: &[u8],
    info: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let private = <Suite as Kem>::PrivateKey::from_bytes(&key.scalar())
        .map_err(|_| CryptoError::InvalidKey)?;
    let encapped = <Suite as Kem>::EncappedKey::from_bytes(enc)
        .map_err(|_| CryptoError::Format("bad HPKE enc"))?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, Suite>(
        &OpModeR::Base,
        &private,
        &encapped,
        info,
        ciphertext,
        aad,
    )
    .map_err(|_| CryptoError::Aead)
}
