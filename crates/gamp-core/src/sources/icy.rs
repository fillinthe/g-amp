//! SHOUTcast/Icecast song titles. A station that is asked for them slips a
//! small metadata block into the audio every `icy-metaint` bytes; this
//! takes the blocks out (the player must only see audio) and remembers the
//! latest `StreamTitle`, which the page shows in Winamp's display.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};

fn titles() -> &'static Mutex<HashMap<String, String>> {
    static T: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// What the station at `url` says is playing now.
pub fn now_playing(url: &str) -> Option<String> {
    titles().lock().unwrap().get(url).cloned()
}

enum State {
    Audio(usize),
    Length,
    Meta(usize),
}

pub struct Strip {
    every: usize,
    state: State,
    meta: Vec<u8>,
    pub title: Option<String>,
}

impl Strip {
    pub fn new(every: usize) -> Strip {
        Strip { every, state: State::Audio(every), meta: Vec::new(), title: None }
    }

    /// The audio in `chunk`, without metadata.
    pub fn feed(&mut self, mut chunk: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(chunk.len());
        while !chunk.is_empty() {
            match self.state {
                State::Audio(left) => {
                    let n = left.min(chunk.len());
                    out.extend_from_slice(&chunk[..n]);
                    chunk = &chunk[n..];
                    self.state = if left == n { State::Length } else { State::Audio(left - n) };
                }
                State::Length => {
                    let len = chunk[0] as usize * 16;
                    chunk = &chunk[1..];
                    self.meta.clear();
                    self.state = if len == 0 { State::Audio(self.every) } else { State::Meta(len) };
                }
                State::Meta(left) => {
                    let n = left.min(chunk.len());
                    self.meta.extend_from_slice(&chunk[..n]);
                    chunk = &chunk[n..];
                    if left == n {
                        if let Some(t) = parse_title(&self.meta) {
                            self.title = Some(t);
                        }
                        self.state = State::Audio(self.every);
                    } else {
                        self.state = State::Meta(left - n);
                    }
                }
            }
        }
        out
    }
}

/// `StreamTitle='Artist - Song';StreamUrl='';` → "Artist - Song".
pub fn parse_title(meta: &[u8]) -> Option<String> {
    let end = meta.iter().position(|b| *b == 0).unwrap_or(meta.len());
    let raw = &meta[..end];
    // Most stations send UTF-8; old ones Latin-1.
    let text = match std::str::from_utf8(raw) {
        Ok(s) => s.to_owned(),
        Err(_) => raw.iter().map(|b| *b as char).collect(),
    };
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let stop = rest.find("';").unwrap_or_else(|| rest.trim_end_matches('\'').len());
    let title = rest[..stop].trim().to_owned();
    (!title.is_empty()).then_some(title)
}

/// Wraps a station's byte stream: audio out, titles noted under `url`.
pub fn strip<S, E>(stream: S, every: usize, url: String) -> impl Stream<Item = Result<Bytes, E>>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    let mut strip = Strip::new(every);
    stream.map(move |chunk| {
        chunk.map(|c| {
            let out = strip.feed(&c);
            if let Some(t) = strip.title.take() {
                titles().lock().unwrap().insert(url.clone(), t);
            }
            Bytes::from(out)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_titles_out_of_the_audio() {
        let meta = b"StreamTitle='Daft Punk - Da Funk';StreamUrl='';";
        let blocks = meta.len().div_ceil(16);
        let mut wire = b"AAAA".to_vec();
        wire.push(blocks as u8);
        wire.extend_from_slice(meta);
        wire.resize(4 + 1 + blocks * 16, 0);
        wire.extend_from_slice(b"BBBB");
        wire.push(0);
        wire.extend_from_slice(b"CC");
        // Fed in awkward pieces, as the network delivers it.
        let mut s = Strip::new(4);
        let mut audio = Vec::new();
        for piece in wire.chunks(3) {
            audio.extend(s.feed(piece));
        }
        assert_eq!(audio, b"AAAABBBBCC");
        assert_eq!(s.title.as_deref(), Some("Daft Punk - Da Funk"));
        assert_eq!(parse_title(b"StreamTitle='';"), None);
        assert_eq!(parse_title(b"StreamTitle='Caf\xe9';"), Some("Café".into()));
    }
}
