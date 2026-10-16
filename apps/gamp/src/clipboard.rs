//! The system clipboard, for pasting addresses and keys into the library.

pub fn get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

pub fn set(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.to_owned());
    }
}
