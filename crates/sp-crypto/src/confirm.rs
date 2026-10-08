//! The 8-digit code the terminal prints and the browser shows before approving
//! a login. Both compute it from the CLI's public keys, so a server that swapped
//! the keys would change the code.

use crate::PublicJwk;

pub fn confirm_code(
    request_id: &str,
    sign_pub: &PublicJwk,
    enc_pub: &PublicJwk,
    jkt: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"skillpouch/v1/cli-confirm");
    hasher.update(request_id.as_bytes());
    hasher.update(sign_pub.canonical().as_bytes());
    hasher.update(enc_pub.canonical().as_bytes());
    hasher.update(jkt.as_bytes());
    let digest = hasher.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest.as_bytes()[..8]);
    let digits = format!("{:08}", u64::from_be_bytes(first) % 100_000_000);
    format!("{}-{}", &digits[..4], &digits[4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwk(x: &str, y: &str) -> PublicJwk {
        PublicJwk {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: x.into(),
            y: y.into(),
        }
    }

    /// Same inputs through `confirmCode` in `@skillpouch/crypto`.
    #[test]
    fn matches_the_web_app() {
        let sign = jwk(
            "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs",
            "9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA",
        );
        let enc = jwk(
            "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU",
            "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0",
        );
        let code = confirm_code(
            "0193a5b2-7c1d-7e8f-9a0b-1c2d3e4f5a6b",
            &sign,
            &enc,
            &sign.thumbprint(),
        );
        assert_eq!(code, "2605-3825");
    }
}
