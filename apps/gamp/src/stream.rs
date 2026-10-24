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
    /// Where in the track this file begins (`X-Gamp-Start`; 0 normally).
    start: f64,
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
        let start = h.get("x-gamp-start").and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(0.0);
        Ok(HttpSource { inner: Mutex::new(Inner { client, url: url.to_owned(), pos: 0, resp: Some(resp) }), len: total, seekable: ranges && total.is_some(), mime, start })
    }

    pub fn start(&self) -> f64 {
        self.start
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
        let (len, seekable) = (self.len, self.seekable);
        let inner = self.inner.get_mut().unwrap();
        // A connection that drops before the end (Wi-Fi, an expired link
        // upstream) picks up where it was, a few times, instead of ending the song.
        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(500 * attempt));
            }
            if inner.resp.is_none() {
                let r = match inner.client.get(&inner.url).header("Range", format!("bytes={}-", inner.pos)).send() {
                    Ok(r) => r,
                    Err(e) if attempt == 3 => return Err(std::io::Error::other(e)),
                    Err(_) => continue,
                };
                if !r.status().is_success() {
                    return Ok(0);
                }
                inner.resp = Some(r);
            }
            let want = !buf.is_empty();
            let short = |pos: u64| want && seekable && len.is_some_and(|l| pos < l);
            match inner.resp.as_mut().unwrap().read(buf) {
                Ok(0) if short(inner.pos) => inner.resp = None,
                Ok(n) => {
                    inner.pos += n as u64;
                    return Ok(n);
                }
                Err(e) if !short(inner.pos) || attempt == 3 => return Err(e),
                Err(_) => inner.resp = None,
            }
        }
        Ok(0)
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
        log::debug!("at {} seek to {target} ({:+})", inner.pos, target as i64 - inner.pos as i64);
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
    use std::io::{Read, Write};

    #[test]
    fn a_dropped_connection_picks_up_where_it_was() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/song.mp3", listener.local_addr().unwrap());
        let body = data.clone();
        std::thread::spawn(move || {
            for (n, conn) in listener.incoming().enumerate() {
                let mut c = conn.unwrap();
                let mut req = [0u8; 2048];
                let got = c.read(&mut req).unwrap();
                let req = String::from_utf8_lossy(&req[..got]).to_string();
                let from: usize = req.split("bytes=").nth(1).and_then(|r| r.split('-').next()?.parse().ok()).unwrap_or(0);
                let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", body.len() - from, body.len() - 1, body.len());
                c.write_all(head.as_bytes()).unwrap();
                // The first answer hangs up a third of the way through.
                let end = if n == 0 { body.len() / 3 } else { body.len() };
                let _ = c.write_all(&body[from..end]);
            }
        });
        let mut src = super::HttpSource::open(&url).unwrap();
        let mut all = Vec::new();
        src.read_to_end(&mut all).unwrap();
        assert!(all == data, "read {} of {} bytes", all.len(), data.len());
    }

    #[test]
    fn extensions() {
        assert_eq!(super::ext_of("/media/spotify/abc.wav").as_deref(), Some("wav"));
        assert_eq!(super::ext_of("/media/remote/x/y?s=1&f=.flac").as_deref(), Some("flac"));
        assert_eq!(super::ext_of("/media/local/8f9e").as_deref(), None);
    }
}
