//! The id the server files this computer's counts under. It names the logs,
//! not the login: the same OS user on the same machine always gets the same
//! id, so logging in again (or reinstalling) re-sends the same logs under it
//! instead of adding them a second time.

use std::path::Path;

/// 32 hex characters derived from the machine id and the user's home folder.
pub fn source_id(home: &Path) -> String {
    let machine = machine_uid::get().unwrap_or_else(|_| gethost().unwrap_or_default());
    derive(&machine, home)
}

fn derive(machine: &str, home: &Path) -> String {
    let mut hasher = blake3::Hasher::new_derive_key("skillpouch/v1/usage-source");
    hasher.update(machine.trim().as_bytes());
    hasher.update(&[0]);
    hasher.update(home.to_string_lossy().as_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

/// Without a machine id the hostname is the next most stable name.
fn gethost() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stable_and_differs_per_user() {
        let ada = derive("machine-1", Path::new("/home/ada"));
        assert_eq!(ada, derive("machine-1\n", Path::new("/home/ada")));
        assert_eq!(ada.len(), 32);
        assert!(ada.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(ada, derive("machine-1", Path::new("/home/bob")));
        assert_ne!(ada, derive("machine-2", Path::new("/home/ada")));
    }
}
