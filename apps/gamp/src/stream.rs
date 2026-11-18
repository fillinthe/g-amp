//! Reading a track over HTTP for the audio engine: the local server
//! fronts every remote source (with byte ranges), so seeking becomes a
//! new ranged request.

use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};

use symphonia::core::io::MediaSource;

/// How far ahead of the decoder a track is read, so a slow moment on the
/// network (a SoundCloud fragment, Wi-Fi) is never heard as a skip.
const AHEAD: u64 = 8 << 20;
/// What is kept behind the decoder (the decoder steps back a little).
const BEHIND: usize = 1 << 20;
/// A seek forward this short waits for the read-ahead instead of reconnecting.
const HOP: u64 = 256 << 10;

/// The read-ahead: bytes `base..base + data.len()` of the track.
#[derive(Default)]
struct Buf {
    base: u64,
    data: Vec<u8>,
    pos: u64,
    /// Bumped by a far seek: the reader for the old place stops.
    generation: u64,
    done: bool,
    error: Option<String>,
    closed: bool,
}

type Shared = Arc<(Mutex<Buf>, Condvar)>;

pub struct HttpSource {
    shared: Shared,
    client: reqwest::blocking::Client,
    url: String,
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
        let src = HttpSource { shared: Arc::default(), client, url: url.to_owned(), len: total, seekable: ranges && total.is_some(), mime, start };
        src.fetch(0, Some(resp));
        Ok(src)
    }

    /// Reads the track into the buffer from where it ends, on a thread of its own.
    fn fetch(&self, generation: u64, resp: Option<reqwest::blocking::Response>) {
        let (shared, client, url, len, seekable) = (self.shared.clone(), self.client.clone(), self.url.clone(), self.len, self.seekable);
        let _ = std::thread::Builder::new().name("gamp-read-ahead".into()).spawn(move || fetch(shared, client, url, len, seekable, generation, resp));
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

impl Drop for HttpSource {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().closed = true;
        self.shared.1.notify_all();
    }
}

fn fetch(shared: Shared, client: reqwest::blocking::Client, url: String, len: Option<u64>, seekable: bool, generation: u64, mut resp: Option<reqwest::blocking::Response>) {
    let (lock, cv) = &*shared;
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        // Wait while far enough ahead.
        let at = {
            let mut b = lock.lock().unwrap();
            loop {
                if b.closed || b.generation != generation {
                    return;
                }
                let end = b.base + b.data.len() as u64;
                if end.saturating_sub(b.pos) < AHEAD {
                    break end;
                }
                b = cv.wait(b).unwrap();
            }
        };
        // A connection that drops before the end (Wi-Fi, an expired link
        // upstream) picks up where it was, a few times, instead of ending the song.
        let short = seekable && len.is_some_and(|l| at < l);
        let mut got = Err(String::new());
        for attempt in 0..6u64 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(500 * attempt));
            }
            if resp.is_none() {
                match client.get(&url).header("Range", format!("bytes={at}-")).send() {
                    Ok(r) if r.status().is_success() => resp = Some(r),
                    Ok(_) => {
                        got = Ok(0);
                        break;
                    }
                    Err(e) => {
                        got = Err(e.to_string());
                        continue;
                    }
                }
            }
            match resp.as_mut().unwrap().read(&mut chunk) {
                Ok(0) if short => resp = None,
                Ok(n) => {
                    got = Ok(n);
                    break;
                }
                Err(e) if !short => {
                    got = Err(e.to_string());
                    break;
                }
                Err(e) => {
                    got = Err(e.to_string());
                    resp = None;
                }
            }
        }
        let mut b = lock.lock().unwrap();
        if b.closed || b.generation != generation {
            return;
        }
        match got {
            Ok(0) => b.done = true,
            Ok(n) => {
                b.data.extend_from_slice(&chunk[..n]);
                // Let go of what the decoder is well past.
                let behind = (b.pos - b.base) as usize;
                if behind > 2 * BEHIND {
                    let drop = behind - BEHIND;
                    b.data.drain(..drop);
                    b.base += drop as u64;
                }
            }
            Err(e) => b.error = Some(e),
        }
        let stop = b.done || b.error.is_some();
        drop(b);
        cv.notify_all();
        if stop {
            return;
        }
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
        if buf.is_empty() {
            return Ok(0);
        }
        let (lock, cv) = &*self.shared;
        let mut b = lock.lock().unwrap();
        loop {
            let end = b.base + b.data.len() as u64;
            if b.pos < end {
                let from = (b.pos - b.base) as usize;
                let n = buf.len().min(b.data.len() - from);
                buf[..n].copy_from_slice(&b.data[from..from + n]);
                b.pos += n as u64;
                drop(b);
                cv.notify_all();
                return Ok(n);
            }
            if b.done {
                return Ok(0);
            }
            if let Some(e) = &b.error {
                return Err(std::io::Error::other(e.clone()));
            }
            b = cv.wait(b).unwrap();
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let (lock, cv) = &*self.shared;
        let mut b = lock.lock().unwrap();
        let target = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => b.pos as i64 + d,
            SeekFrom::End(d) => self.len.ok_or_else(|| std::io::Error::other("unknown length"))? as i64 + d,
        };
        if target < 0 {
            return Err(std::io::Error::other("seek before start"));
        }
        let target = target as u64;
        let end = b.base + b.data.len() as u64;
        log::debug!("at {} seek to {target} ({:+})", b.pos, target as i64 - b.pos as i64);
        if target >= b.base && (target <= end + HOP || b.done) {
            // In the buffer, or a short hop the read-ahead is about to cover.
            b.pos = target;
        } else {
            // Somewhere else: a new request from there.
            b.generation += 1;
            let generation = b.generation;
            (b.base, b.pos, b.done, b.error) = (target, target, false, None);
            b.data.clear();
            drop(b);
            self.fetch(generation, None);
        }
        cv.notify_all();
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

    /// Reading ahead keeps every byte in place across near and far seeks.
    #[test]
    fn seeks_read_the_right_bytes() {
        use std::io::{Seek, SeekFrom};
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/song.mp3", listener.local_addr().unwrap());
        let body = data.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut c = conn.unwrap();
                    let mut req = [0u8; 2048];
                    let got = c.read(&mut req).unwrap();
                    let req = String::from_utf8_lossy(&req[..got]).to_string();
                    let from: usize = req.split("bytes=").nth(1).and_then(|r| r.split('-').next()?.parse().ok()).unwrap_or(0);
                    let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", body.len() - from, body.len() - 1, body.len());
                    let _ = c.write_all(head.as_bytes());
                    let _ = c.write_all(&body[from..]);
                });
            }
        });
        let mut src = super::HttpSource::open(&url).unwrap();
        let mut buf = vec![0u8; 5000];
        for at in [0u64, 100, 4000, 2_900_000, 10, 1_500_000, 1_500_100] {
            src.seek(SeekFrom::Start(at)).unwrap();
            src.read_exact(&mut buf).unwrap();
            assert!(buf[..] == data[at as usize..at as usize + 5000], "at {at}");
        }
        src.seek(SeekFrom::Start(2_999_000)).unwrap();
        let mut rest = Vec::new();
        src.read_to_end(&mut rest).unwrap();
        assert_eq!(rest.len(), 1000);
    }

    #[test]
    fn extensions() {
        assert_eq!(super::ext_of("/media/spotify/abc.wav").as_deref(), Some("wav"));
        assert_eq!(super::ext_of("/media/remote/x/y?s=1&f=.flac").as_deref(), Some("flac"));
        assert_eq!(super::ext_of("/media/local/8f9e").as_deref(), None);
    }
}
