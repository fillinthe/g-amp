//! Shared HTTP pieces: one client, byte-range parsing, JSON errors.

use std::sync::OnceLock;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

fn builder() -> reqwest::ClientBuilder {
    // No overall timeout: streams last as long as the music.
    reqwest::Client::builder()
        .user_agent("G-Amp/0.1")
        .connect_timeout(std::time::Duration::from_secs(15))
}

pub fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| builder().build().expect("http client"))
}

/// For a server you chose to trust with a self-signed certificate (a NAS
/// on https://192.168.x.x:5001, say).
pub fn insecure_client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| builder().danger_accept_invalid_certs(true).build().expect("http client"))
}

/// A request's `Range: bytes=…` against a resource of `size` bytes:
/// `Ok(None)` for the whole thing, `Err(())` when unsatisfiable.
pub fn parse_range(header: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(h) = header else { return Ok(None) };
    let Some(spec) = h.strip_prefix("bytes=") else { return Ok(None) };
    let Some((a, b)) = spec.split_once('-') else { return Ok(None) };
    if size == 0 {
        return Err(());
    }
    let (start, end) = match (a.trim(), b.trim()) {
        ("", "") => return Ok(None),
        ("", n) => {
            let n: u64 = n.parse().map_err(|_| ())?;
            (size.saturating_sub(n), size - 1)
        }
        (s, "") => (s.parse().map_err(|_| ())?, size - 1),
        (s, e) => (s.parse().map_err(|_| ())?, e.parse::<u64>().map_err(|_| ())?.min(size - 1)),
    };
    if start > end || start >= size { Err(()) } else { Ok(Some((start, end))) }
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, axum::Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

pub fn err(status: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(status, msg.into())
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 100), Ok(None));
        assert_eq!(parse_range(Some("bytes=0-9"), 100), Ok(Some((0, 9))));
        assert_eq!(parse_range(Some("bytes=90-"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_range(Some("bytes=50-500"), 100), Ok(Some((50, 99))));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Err(()));
    }
}
