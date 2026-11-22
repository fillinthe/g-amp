//! Anonymous usage stats: when Gamp starts it sends its version, the
//! system (macOS, Windows, Linux), the processor type and the language,
//! under a random id made for this install. Never music, files, accounts or
//! server addresses; the server keeps no IP addresses. Gamp says so on the
//! first start; Options turns it off (which also erases what was sent) or
//! deletes everything Gamp keeps.

use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

const URL: &str = "https://gampapp.com/api/stats";

/// A random 32-hex-digit id for this install.
pub fn new_id() -> String {
    let n = |_| RandomState::new().hash_one(std::time::SystemTime::now());
    format!("{:016x}{:016x}", n(0), n(1))
}

fn post(path: &str, body: serde_json::Value) {
    // Tests stay offline, except the end-to-end run against the live server.
    if cfg!(test) && std::env::var_os("GAMP_E2E").is_none() {
        return;
    }
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(8)).build();
    if let Ok(c) = client {
        let _ = c.post(format!("{URL}{path}")).header("Content-Type", "application/json").body(body.to_string()).send();
    }
}

/// Sends this start's check-in, in the background.
pub fn send(id: String, lang: &str) {
    let body = serde_json::json!({
        "id": id,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "lang": lang,
    });
    let _ = std::thread::Builder::new().name("gamp-stats".into()).spawn(move || post("", body));
}

/// Erases this install's stats on the server (waits for it, a few seconds at most).
pub fn forget(id: &str) {
    if !id.is_empty() {
        post("/delete", serde_json::json!({ "id": id }));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_are_random_hex() {
        let (a, b) = (super::new_id(), super::new_id());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
