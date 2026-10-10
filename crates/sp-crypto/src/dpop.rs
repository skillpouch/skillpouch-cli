//! DPoP proofs (RFC 9449), as `verifyDpopProof` on the server checks them.

use crate::{Result, SignKey, b64};
use p256::ecdsa::Signature;
use p256::ecdsa::signature::Signer;
use serde_json::json;
use sha2::{Digest, Sha256};

pub struct ProofInput<'a> {
    pub method: &'a str,
    /// Absolute URL without query or fragment.
    pub url: &'a str,
    /// Unix seconds.
    pub issued_at: u64,
    pub nonce: Option<&'a str>,
    /// The access token sent alongside, bound through `ath`.
    pub access_token: Option<&'a str>,
}

pub fn proof(key: &SignKey, input: &ProofInput<'_>) -> Result<String> {
    let header = json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": key.public_jwk()? });
    let mut claims = json!({
        "jti": uuid::Uuid::new_v4().to_string(),
        "htm": input.method,
        // The query and fragment are not part of `htu` (RFC 9449 §4.2).
        "htu": input.url.split(['?', '#']).next().unwrap_or(input.url),
        "iat": input.issued_at,
    });
    if let Some(nonce) = input.nonce {
        claims["nonce"] = json!(nonce);
    }
    if let Some(token) = input.access_token {
        claims["ath"] = json!(access_token_hash(token));
    }
    let signing_input = format!(
        "{}.{}",
        b64::encode(header.to_string()),
        b64::encode(claims.to_string())
    );
    let signature: Signature = key.signing_key().sign(signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        b64::encode(signature.to_bytes())
    ))
}

/// `ath`: base64url(SHA-256(access token)).
pub fn access_token_hash(token: &str) -> String {
    b64::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier;
    use serde_json::Value;

    fn part(text: &str) -> Option<Value> {
        serde_json::from_slice(&b64::decode(text)?).ok()
    }

    #[test]
    fn signs_a_verifiable_proof_with_every_claim() -> Result<()> {
        let key = SignKey::generate()?;
        let token = proof(
            &key,
            &ProofInput {
                method: "POST",
                url: "https://app.skillpouch.net/v1/cli-auth/token",
                issued_at: 1_700_000_000,
                nonce: Some("42.abc"),
                access_token: Some("token"),
            },
        )?;
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        let header = part(parts[0]).unwrap_or_default();
        assert_eq!(header["typ"], "dpop+jwt");
        assert_eq!(header["alg"], "ES256");
        assert_eq!(
            header["jwk"],
            serde_json::to_value(key.public_jwk()?).unwrap_or_default()
        );

        let claims = part(parts[1]).unwrap_or_default();
        assert_eq!(claims["htm"], "POST");
        assert_eq!(
            claims["htu"],
            "https://app.skillpouch.net/v1/cli-auth/token"
        );
        assert_eq!(claims["iat"], 1_700_000_000);
        assert_eq!(claims["nonce"], "42.abc");
        assert_eq!(claims["ath"], access_token_hash("token"));
        assert!(claims["jti"].as_str().is_some_and(|jti| jti.len() >= 8));

        let signature = b64::decode(parts[2]).and_then(|bytes| Signature::from_slice(&bytes).ok());
        let verifying = VerifyingKey::from(key.signing_key());
        let signed = format!("{}.{}", parts[0], parts[1]);
        assert!(signature.is_some_and(|s| verifying.verify(signed.as_bytes(), &s).is_ok()));
        Ok(())
    }

    #[test]
    fn leaves_out_optional_claims() -> Result<()> {
        let key = SignKey::generate()?;
        let input = ProofInput {
            method: "GET",
            url: "http://localhost/v1/me",
            issued_at: 1,
            nonce: None,
            access_token: None,
        };
        let token = proof(&key, &input)?;
        let claims = token.split('.').nth(1).and_then(part).unwrap_or_default();
        assert!(claims.get("nonce").is_none() && claims.get("ath").is_none());
        // Every proof has its own jti.
        assert_ne!(token, proof(&key, &input)?);
        Ok(())
    }

    #[test]
    fn the_url_is_signed_without_its_query() -> Result<()> {
        let key = SignKey::generate()?;
        let token = proof(
            &key,
            &ProofInput {
                method: "GET",
                url: "http://localhost/v1/pouches/p/artifacts?limit=200",
                issued_at: 1,
                nonce: None,
                access_token: None,
            },
        )?;
        let claims = token.split('.').nth(1).and_then(part).unwrap_or_default();
        assert_eq!(claims["htu"], "http://localhost/v1/pouches/p/artifacts");
        Ok(())
    }
}
