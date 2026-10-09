//! Sealed objects, the one ciphertext layout for blobs, revision envelopes
//! and encrypted names:
//!
//! ```text
//! sealed = 0x01 ‖ u32_be(key_gen) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, pad(plaintext), header ‖ aad)
//! ```
//!
//! The plaintext is Padmé-padded: `plaintext ‖ 0x80 ‖ 0x00…`.

use crate::{CryptoError, Result, random_bytes};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

const FORMAT_V1: u8 = 1;
const HEADER: usize = 1 + 4 + 24;
const TAG: usize = 16;

/// The key generation a sealed object was made with, read without decrypting.
pub fn key_gen(sealed: &[u8]) -> Result<u32> {
    if sealed.len() < HEADER + TAG || sealed[0] != FORMAT_V1 {
        return Err(CryptoError::Format("not a v1 sealed object"));
    }
    Ok(u32::from_be_bytes([
        sealed[1], sealed[2], sealed[3], sealed[4],
    ]))
}

pub fn open(key: &[u8; 32], sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    key_gen(sealed)?;
    let (header, body) = sealed.split_at(HEADER);
    let nonce = XNonce::try_from(&header[5..]).map_err(|_| CryptoError::Format("bad nonce"))?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let padded = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: body,
                aad: &[header, aad].concat(),
            },
        )
        .map_err(|_| CryptoError::Aead)?;
    unpad(padded)
}

fn unpad(mut padded: Vec<u8>) -> Result<Vec<u8>> {
    while padded.last() == Some(&0) {
        padded.pop();
    }
    if padded.pop() != Some(0x80) {
        return Err(CryptoError::Format("padding marker missing"));
    }
    Ok(padded)
}

/// Seals `plaintext` under `key` with a random nonce.
pub fn seal(key: &[u8; 32], key_gen: u32, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let nonce: [u8; 24] = random_bytes()?;
    let header = [&[FORMAT_V1][..], &key_gen.to_be_bytes(), &nonce].concat();
    let body = XChaCha20Poly1305::new(key.into())
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &pad(plaintext),
                aad: &[&header[..], aad].concat(),
            },
        )
        .map_err(|_| CryptoError::Aead)?;
    Ok([header, body].concat())
}

/// Padmé length for `length` bytes: sizes only leak a coarse bucket.
pub fn padme_length(length: usize) -> usize {
    if length < 2 {
        return length;
    }
    let e = usize::BITS - 1 - length.leading_zeros();
    let s = u32::BITS - 1 - e.leading_zeros() + 1;
    let last_bits = e - s;
    let mask = (1usize << last_bits) - 1;
    (length + mask) & !mask
}

fn pad(plaintext: &[u8]) -> Vec<u8> {
    let mut padded = vec![0u8; padme_length(plaintext.len() + 1)];
    padded[..plaintext.len()].copy_from_slice(plaintext);
    padded[plaintext.len()] = 0x80;
    padded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_what_it_seals_and_refuses_tampering() {
        let key = [9u8; 32];
        let sealed = seal(&key, 3, b"hello", b"aad").unwrap_or_default();
        assert_eq!(key_gen(&sealed).ok(), Some(3));
        assert_eq!(open(&key, &sealed, b"aad").ok(), Some(b"hello".to_vec()));
        assert!(open(&key, &sealed, b"other").is_err());
        let mut relabelled = sealed.clone();
        relabelled[4] = 4;
        assert!(open(&key, &relabelled, b"aad").is_err());
        assert!(key_gen(&[2u8; 64]).is_err());
    }

    #[test]
    fn padme_matches_the_web_client() {
        // Values from `padmeLength` in @skillpouch/crypto.
        for (length, padded) in [
            (0, 0),
            (1, 1),
            (2, 2),
            (9, 10),
            (100, 104),
            (1000, 1024),
            (5000, 5120),
        ] {
            assert_eq!(padme_length(length), padded, "{length}");
        }
        let sealed = seal(&[1u8; 32], 1, &[5u8; 99], b"").unwrap_or_default();
        // header 29 + padded 104 + tag 16
        assert_eq!(sealed.len(), 29 + 104 + 16);
    }
}
