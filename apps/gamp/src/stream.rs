//! Reading a track over HTTP for the audio engine: the local server
//! fronts every remote source (with byte ranges), so seeking becomes a
//! new ranged request.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use symphonia::core::io::MediaSource;

struct Inner {
    client: reqwest::blocking::Client,
    url: String,
    pos: u64,
    resp: Option<reqwest::blocking::Response>,
}

pub struct HttpSource {
    inner: Mutex<Inner>,
    len: Option<u64>,
    seekable: bool,
    mime: Option<String>,
}

fn client() -> reqwest::blocking::Client {
    // No overall timeout: a stream lasts as long as the music.
    reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).timeout(None).build().expect("http client")
}

impl HttpSource {
    /// Opens `url`, learning its length and whether it can seek.
    pub fn open(url: &str) -> anyhow::Result<HttpSource> {
        let client = client();
        let resp = client.get(url).header("Range", "bytes=0-").send()?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            let why = serde_error(&body).unwrap_or_else(|| status.to_string());
            anyhow::bail!("{why}");
        }
        let h = resp.headers();
        let total = h
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next()?.parse::<u64>().ok())
            .or_else(|| h.get("content-length").and_then(|v| v.to_str().ok()?.parse().ok()));
        let ranges = status == reqwest::StatusCode::PARTIAL_CONTENT || h.get("accept-ranges").is_some_and(|v| v.as_bytes() == b"bytes");
        let mime = h.get("content-type").and_then(|v| v.to_str().ok()).map(|m| m.split(';').next().unwrap_or("").trim().to_lowercase());
        Ok(HttpSource { inner: Mutex::new(Inner { client, url: url.to_owned(), pos: 0, resp: Some(resp) }), len: total, seekable: ranges && total.is_some(), mime })
    }

    /// The format the server says it sent, when that's specific.
    pub fn ext(&self) -> Option<String> {
        let e = match self.mime.as_deref()? {
            "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "m4a",
            "audio/flac" | "audio/x-flac" => "flac",
            "audio/ogg" => "ogg",
            _ => return None,
        };
        Some(e.to_owned())
    }
}

/// `{"error": "..."}` from the local server.
fn serde_error(body: &str) -> Option<String> {
    let i = body.find("\"error\":\"")? + 9;
    let rest = &body[i..];
    Some(rest[..rest.find('"')?].to_owned())
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let inner = self.inner.get_mut().unwrap();
        if inner.resp.is_none() {
            let r = inner
                .client
                .get(&inner.url)
                .header("Range", format!("bytes={}-", inner.pos))
                .send()
                .map_err(std::io::Error::other)?;
            if !r.status().is_success() {
                return Ok(0);
            }
            inner.resp = Some(r);
        }
        let n = inner.resp.as_mut().unwrap().read(buf)?;
        inner.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let len = self.len;
        let inner = self.inner.get_mut().unwrap();
        let target = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => inner.pos as i64 + d,
            SeekFrom::End(d) => len.ok_or_else(|| std::io::Error::other("unknown length"))? as i64 + d,
        };
        if target < 0 {
            return Err(std::io::Error::other("seek before start"));
        }
        let target = target as u64;
        if target != inner.pos {
            // Short hops forward: read through instead of a new request.
            if target > inner.pos && target - inner.pos <= 32 * 1024 && inner.resp.is_some() {
                let mut skip = vec![0u8; (target - inner.pos) as usize];
                let r = inner.resp.as_mut().unwrap();
                r.read_exact(&mut skip)?;
            } else {
                inner.resp = None;
            }
            inner.pos = target;
        }
        Ok(target)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        self.seekable
    }
    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

/// The file type a URL points at, for the decoder's hint.
pub fn ext_of(url: &str) -> Option<String> {
    if let Some(i) = url.find("f=.") {
        let rest = &url[i + 3..];
        return Some(rest.split(['&', '#']).next().unwrap_or("").to_lowercase());
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    last.rsplit_once('.').map(|(_, e)| e.to_lowercase()).filter(|e| e.len() <= 4)
}

#[cfg(test)]
mod tests {
    #[test]
    fn extensions() {
        assert_eq!(super::ext_of("/media/spotify/abc.wav").as_deref(), Some("wav"));
        assert_eq!(super::ext_of("/media/remote/x/y?s=1&f=.flac").as_deref(), Some("flac"));
        assert_eq!(super::ext_of("/media/local/8f9e").as_deref(), None);
    }
}
