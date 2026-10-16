//! Tags for music in plain file storage (WebDAV, Dropbox), which lists
//! only file names. The start of each file holds its tag (ID3v2, FLAC's
//! Vorbis comments, Ogg's), and for FLAC and most MP3s also the length; so
//! only that much is fetched, never the whole song.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use futures_util::StreamExt;
use lofty::config::ParseOptions;
use lofty::prelude::*;

use super::*;

const HEAD: u64 = 128 * 1024;

#[derive(Clone, Default)]
struct Found {
    title: String,
    artist: String,
    album: String,
    track: Option<u32>,
    duration: Option<f64>,
}

/// Read again only after a restart.
fn cache() -> &'static Mutex<HashMap<String, Option<Found>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Found>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The file's first bytes, with the rest standing in as zeros, so a tag
/// reader that looks at the length (CBR MP3) or seeks to the end (ID3v1)
/// still works.
struct Sparse {
    head: Vec<u8>,
    len: u64,
    pos: u64,
}

impl Read for Sparse {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(self.len - self.pos) as usize;
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = self.head.get(self.pos as usize + i).copied().unwrap_or(0);
        }
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Sparse {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if p < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

fn parse(head: Vec<u8>, len: u64, ext: &str) -> Option<Found> {
    let read = |props: bool| {
        let r = Sparse { head: head.clone(), len, pos: 0 };
        let mut probe = lofty::probe::Probe::new(r).options(ParseOptions::new().read_properties(props));
        if let Some(t) = lofty::file::FileType::from_ext(ext) {
            probe = probe.set_file_type(t);
        } else {
            probe = probe.guess_file_type().ok()?;
        }
        probe.read().ok()
    };
    let (file, props) = match read(true) {
        Some(f) => (f, true),
        None => (read(false)?, false),
    };
    let mut out = Found::default();
    if props {
        let mut secs = file.properties().duration().as_secs_f64();
        // An MP3 without a length header gets measured over the bytes
        // fetched only; work it out from the file size and bitrate.
        if let Some(kbps) = file.properties().audio_bitrate().filter(|k| *k > 0) {
            let bytes_heard = secs * kbps as f64 * 1000.0 / 8.0;
            if len > HEAD * 3 / 2 && bytes_heard < HEAD as f64 * 1.2 {
                secs = len as f64 * 8.0 / (kbps as f64 * 1000.0);
            }
        }
        // A length from zeros past the head would be made up; trust only
        // plausible ones.
        if secs > 1.0 && secs < 6.0 * 3600.0 {
            out.duration = Some(secs.round());
        }
    }
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        out.title = tag.title().map(|v| v.trim().to_owned()).unwrap_or_default();
        out.artist = tag.artist().map(|v| v.trim().to_owned()).unwrap_or_default();
        if out.artist.is_empty() {
            out.artist = tag.get_string(&lofty::tag::ItemKey::AlbumArtist).unwrap_or_default().to_owned();
        }
        out.album = tag.album().map(|v| v.trim().to_owned()).unwrap_or_default();
        out.track = tag.track();
    }
    Some(out)
}

async fn fetch(up: Upstream) -> Option<Found> {
    let http = if up.insecure { insecure_client() } else { client() };
    let mut req = http.get(&up.url).header("Range", format!("bytes=0-{}", HEAD - 1)).timeout(std::time::Duration::from_secs(8));
    for (k, v) in &up.headers {
        req = req.header(k, v);
    }
    let res = req.send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    // "bytes 0-131071/4123456" → the whole file's size.
    let total = res
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next()?.parse::<u64>().ok())
        .or(res.content_length())?;
    let mut head = Vec::new();
    let mut body = res.bytes_stream();
    while let Some(Ok(chunk)) = body.next().await {
        head.extend_from_slice(&chunk);
        if head.len() as u64 >= HEAD {
            break;
        }
    }
    let ext = up.url.split(['?', '#']).next().unwrap_or("").rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    tokio::task::spawn_blocking(move || parse(head, total, &ext)).await.ok().flatten()
}

/// Fills in title, artist, album, track number and length for the tracks
/// in a listing, a few files at a time. `upstream` says where each lives.
pub async fn enrich<F, Fut>(sid: &str, items: &mut [Item], upstream: F)
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Option<Upstream>>,
{
    let todo: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.kind == "track")
        .take(400)
        .map(|(n, i)| (n, format!("{sid}\u{1f}{}", i.id)))
        .collect();
    let missing: Vec<(usize, String)> = {
        let c = cache().lock().unwrap();
        todo.iter().filter(|(_, k)| !c.contains_key(k)).cloned().collect()
    };
    let found: Vec<(String, Option<Found>)> = futures_util::stream::iter(missing)
        .map(|(n, key)| {
            let id = items[n].id.clone();
            let up = upstream(id);
            async move {
                let f = match up.await {
                    Some(up) => fetch(up).await,
                    None => None,
                };
                (key, f)
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;
    let mut c = cache().lock().unwrap();
    for (k, f) in found {
        c.insert(k, f);
    }
    for (n, key) in todo {
        let Some(Some(f)) = c.get(&key) else { continue };
        let item = &mut items[n];
        if !f.title.is_empty() {
            item.title = f.title.clone();
        }
        if !f.artist.is_empty() {
            item.artist = f.artist.clone();
        }
        if !f.album.is_empty() {
            item.album = f.album.clone();
        }
        if f.track.is_some() {
            item.track_no = f.track;
        }
        if f.duration.is_some() {
            item.duration = f.duration;
        }
    }
}
