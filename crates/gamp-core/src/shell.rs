//! The desktop shell's shared state.
//!
//! On the desktop, Winamp gets its own borderless, transparent window,
//! fitted to the Winamp windows it shows. The page reports where those
//! windows are (in CSS pixels) and whether a mouse button is held, so the
//! transparent gaps between them (say beside a short main window and a tall
//! library) let clicks through to whatever is behind.

use std::sync::{Mutex, OnceLock};

#[derive(Default, Clone)]
pub struct Hit {
    pub rects: Vec<[f64; 4]>,
    pub pressed: bool,
}

pub fn hit() -> &'static Mutex<Hit> {
    static H: OnceLock<Mutex<Hit>> = OnceLock::new();
    H.get_or_init(Default::default)
}

/// How the page is being shown: "floating" (the desktop app), "mobile",
/// or "browser" (the --server mode, opened in any browser).
static SHELL: OnceLock<&'static str> = OnceLock::new();

pub fn set_shell(shell: &'static str) {
    let _ = SHELL.set(shell);
}

pub fn shell() -> &'static str {
    SHELL.get().copied().unwrap_or("browser")
}

type Control = Box<dyn Fn(&str, &serde_json::Value) + Send + Sync>;
static CONTROL: OnceLock<Control> = OnceLock::new();

/// The desktop shell's handler for page requests (quit, minimize, drag,
/// frame).
pub fn set_control(f: Control) {
    let _ = CONTROL.set(f);
}

pub fn control(action: &str, body: &serde_json::Value) -> bool {
    match CONTROL.get() {
        Some(f) => {
            f(action, body);
            true
        }
        None => false,
    }
}

/// Where a sign-in lands. The desktop app signs in through your web
/// browser (password managers and two-step codes work there), so the
/// browser tab says it's done and the app window comes forward; on a
/// phone or in a browser it goes back to the player.
pub fn after_sign_in(service: &str, result: Result<(), String>) -> String {
    let service_q = urlencoding::encode(service);
    if shell() == "floating" {
        control("focus", &serde_json::Value::Null);
        return match result {
            Ok(()) => format!("/signed-in.html?service={service_q}"),
            Err(e) => format!("/signed-in.html?service={service_q}&error={}", urlencoding::encode(&e)),
        };
    }
    match result {
        Ok(()) => format!("/?connected={}", urlencoding::encode(&service.to_lowercase())),
        Err(e) => format!("/?error={}", urlencoding::encode(&e)),
    }
}

pub fn inside(hit: &Hit, x: f64, y: f64) -> bool {
    hit.pressed || hit.rects.iter().any(|[l, t, w, h]| x >= *l && y >= *t && x < l + w && y < t + h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_land_only_on_windows() {
        let h = Hit { rects: vec![[10.0, 10.0, 275.0, 116.0]], pressed: false };
        assert!(inside(&h, 10.0, 10.0));
        assert!(inside(&h, 284.0, 125.0));
        assert!(!inside(&h, 285.0, 50.0));
        assert!(!inside(&h, 5.0, 50.0));
        // A held button keeps the window's grip during a fast drag.
        assert!(inside(&Hit { pressed: true, ..h }, 900.0, 900.0));
    }
}
