//! base64url without padding, as used throughout the API.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub fn encode(bytes: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}
