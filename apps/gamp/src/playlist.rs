//! The play queue: entries, the current one, shuffle and repeat.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub artist: String,
    pub title: String,
    pub duration: Option<f64>,
}

impl Entry {
    /// Reads the tags, falling back to the file name.
    pub fn read(path: &Path) -> Entry {
        use lofty::prelude::*;
        let mut e = Entry { path: path.to_path_buf(), artist: String::new(), title: String::new(), duration: None };
        if let Ok(file) = lofty::read_from_path(path) {
            let d = file.properties().duration().as_secs_f64();
            if d > 0.0 {
                e.duration = Some(d);
            }
            if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
                e.title = tag.title().map(|t| t.trim().to_owned()).unwrap_or_default();
                e.artist = tag.artist().map(|t| t.trim().to_owned()).unwrap_or_default();
            }
        }
        if e.title.is_empty() {
            e.title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        }
        e
    }

    /// "Artist - Title", or just the title.
    pub fn display(&self) -> String {
        if self.artist.is_empty() { self.title.clone() } else { format!("{} - {}", self.artist, self.title) }
    }
}

pub const AUDIO: &[&str] = &["mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "wav", "aif", "aiff", "alac", "mka", "webm", "caf"];

pub fn is_audio(p: &Path) -> bool {
    p.extension().map(|e| AUDIO.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

/// Files, and every audio file inside folders, in name order.
pub fn expand(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut inner: Vec<PathBuf> = std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            inner.sort_by_key(|p| p.to_string_lossy().to_lowercase());
            inner.retain(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')));
            out.extend(expand(&inner));
        } else if is_audio(p) {
            out.push(p.clone());
        }
    }
    out
}

#[derive(Default)]
pub struct Playlist {
    pub entries: Vec<Entry>,
    pub current: usize,
    pub shuffle: bool,
    pub repeat: bool,
    /// The order to play in when shuffling.
    order: Vec<usize>,
}

impl Playlist {
    pub fn set(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.current = 0;
        self.reshuffle();
    }

    pub fn append(&mut self, entries: Vec<Entry>) {
        self.entries.extend(entries);
        self.reshuffle();
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.current)
    }

    pub fn reshuffle(&mut self) {
        let n = self.entries.len();
        self.order = (0..n).collect();
        if self.shuffle && n > 1 {
            // A small xorshift is plenty for shuffling a playlist.
            let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
            for i in (1..n).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                self.order.swap(i, (seed % (i as u64 + 1)) as usize);
            }
        }
    }

    fn pos(&self) -> usize {
        self.order.iter().position(|i| *i == self.current).unwrap_or(0)
    }

    /// The entry after `current`, wrapping only with repeat.
    pub fn after(&self, index: usize) -> Option<usize> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        let pos = self.order.iter().position(|i| *i == index).unwrap_or(0);
        if pos + 1 < n {
            Some(self.order[pos + 1])
        } else if self.repeat {
            Some(self.order[0])
        } else {
            None
        }
    }

    pub fn next(&mut self) -> bool {
        match self.after(self.current) {
            Some(i) => {
                self.current = i;
                true
            }
            None => false,
        }
    }

    pub fn previous(&mut self) -> bool {
        let n = self.entries.len();
        if n == 0 {
            return false;
        }
        let pos = self.pos();
        self.current = if pos > 0 { self.order[pos - 1] } else if self.repeat { self.order[n - 1] } else { return false };
        true
    }
}
