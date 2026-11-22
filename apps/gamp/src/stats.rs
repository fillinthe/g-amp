//! Usage and problem reports, the same as the other G Workshop apps (GCarve,
//! GBurn, GSketch) and to the same server: a count each time Gamp opens
//! (with its version, system and language) and a short report when
//! something goes wrong (the message without file names, paths or
//! addresses, Gamp's version and the system). The server keeps day counts
//! and problem reports (30 days), nothing that says who sent them: no
//! account, no id, no IP address. Gamp says so on the first start;
//! Options turns it off, and Delete my data… erases what Gamp keeps.

use std::time::Duration;

/// G Workshop's server, as the other apps use.
const BASE: &str = "https://gcarve.app";
/// Where Gamp 0.3.30 kept per-install stats; still erases them on request.
const OLD: &str = "https://gampapp.com/api/stats/delete";

fn post(url: &str, body: serde_json::Value) {
    // Tests stay offline, except the end-to-end run against the live server.
    if cfg!(test) && std::env::var_os("GAMP_E2E").is_none() {
        return;
    }
    let client = reqwest::blocking::Client::builder().user_agent(concat!("Gamp/", env!("CARGO_PKG_VERSION"))).timeout(Duration::from_secs(8)).build();
    if let Ok(c) = client {
        let _ = c.post(url).header("Content-Type", "application/json").body(body.to_string()).send();
    }
}

fn background(url: String, body: serde_json::Value) {
    let _ = std::thread::Builder::new().name("gamp-reports".into()).spawn(move || post(&url, body));
}

fn system() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Gamp opened: one count for the day (detail: version, system, language).
pub fn opened(lang: &str) {
    let detail = format!("{} {} {lang}", env!("CARGO_PKG_VERSION"), system());
    background(format!("{BASE}/api/event"), serde_json::json!({ "name": "gamp-open", "detail": detail }));
}

/// Something went wrong (the English message), with names and addresses taken out.
pub fn problem(message: &str) {
    let body = serde_json::json!({
        "source": "app",
        "message": format!("Gamp: {}", scrub(message)),
        "where": format!("Gamp v{} {}", env!("CARGO_PKG_VERSION"), system()),
    });
    background(format!("{BASE}/api/error"), body);
}

/// Erases what Gamp 0.3.30 sent under this install's id (waits a few seconds at most).
pub fn forget_old(id: &str) {
    if !id.is_empty() {
        post(OLD, serde_json::json!({ "id": id }));
    }
}

/// A message without file names, paths, addresses or accounts: any word
/// with a `/`, `\`, `@`, a dot inside or a `:` inside becomes "…". A
/// place in Gamp's own source (`apps/gamp/src/app.rs:12:5`) stays.
pub fn scrub(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let t = w.trim_end_matches(|c: char| matches!(c, '.' | ',' | ':' | ';' | ')' | '!' | '?' | '"' | '\''));
            let t = t.trim_start_matches(['(', '"', '\'']);
            let ours = (t.starts_with("apps/") || t.starts_with("crates/")) && t.contains(".rs");
            if !ours && t.contains(['/', '\\', '@', '.', ':']) { "…" } else { w }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn reports_carry_no_names_or_addresses() {
        assert_eq!(scrub("CAN'T REACH http://192.168.1.5:32400/library (timed out)"), "CAN'T REACH … (timed out)");
        assert_eq!(scrub("No such file: /Users/ann/Music/Song.mp3."), "No such file: …");
        assert_eq!(scrub(r"denied C:\Users\bob\x and me@mail.com, nas.local:5000"), "denied … and … …");
        assert_eq!(scrub("panicked at apps/gamp/src/app.rs:12:5: index out of range"), "panicked at apps/gamp/src/app.rs:12:5: index out of range");
    }
}
