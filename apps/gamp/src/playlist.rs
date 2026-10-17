//! The play queue: entries, the current one, shuffle and repeat.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// A file; empty for network tracks.
    pub path: PathBuf,
    /// A network track: its address on the local server (/media/...).
    pub url: Option<String>,
    /// Endless (radio).
    pub live: bool,
    pub artist: String,
    pub title: String,
    pub duration: Option<f64>,
}

impl Entry {
    /// Reads the tags, falling back to the file name.
    pub fn read(path: &Path) -> Entry {
        use lofty::prelude::*;
        let mut e = Entry { path: path.to_path_buf(), ..Default::default() };
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

    /// A track from the library's sources.
    pub fn remote(url: String, title: String, artist: String, duration: Option<f64>, live: bool) -> Entry {
        Entry { url: Some(url), title, artist, duration, live, ..Default::default() }
    }

    /// The same track (for keeping a place in the queue).
    pub fn key(&self) -> String {
        self.url.clone().unwrap_or_else(|| self.path.to_string_lossy().into_owned())
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

/// A line of a saved .m3u/.m3u8/.pls playlist.
#[derive(Clone, Debug, PartialEq)]
pub enum Listed {
    /// A file on disk.
    File(PathBuf),
    /// A stream or web address, with what the playlist says about it.
    Link { url: String, title: String, duration: Option<f64> },
}

/// What an .m3u/.m3u8 or .pls playlist lists: files (relative paths are
/// relative to the playlist) and addresses, with the titles and lengths
/// the playlist gives them.
pub fn read_list(path: &Path) -> Vec<Listed> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let base = path.parent().unwrap_or(Path::new("."));
    let pls = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pls")) || text.trim_start().to_ascii_lowercase().starts_with("[playlist]");
    if pls { parse_pls(&text, base) } else { parse_m3u(&text, base) }
}

fn listed(place: &str, base: &Path, title: String, duration: Option<f64>) -> Option<Listed> {
    if place.contains("://") {
        let title = if title.is_empty() { place.to_owned() } else { title };
        return Some(Listed::Link { url: place.to_owned(), title, duration });
    }
    let p = PathBuf::from(place);
    let p = if p.is_absolute() { p } else { base.join(p) };
    is_audio(&p).then_some(Listed::File(p))
}

fn parse_m3u(text: &str, base: &Path) -> Vec<Listed> {
    let mut out = Vec::new();
    let mut title = String::new();
    let mut duration = None;
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(info) = l.strip_prefix("#EXTINF:") {
            let (secs, name) = info.split_once(',').unwrap_or((info, ""));
            duration = secs.trim().parse::<f64>().ok().filter(|d| *d > 0.0);
            title = name.trim().to_owned();
        } else if !l.starts_with('#') {
            out.extend(listed(l, base, std::mem::take(&mut title), duration.take()));
        }
    }
    out
}

fn parse_pls(text: &str, base: &Path) -> Vec<Listed> {
    // File1=, Title1=, Length1= in any order.
    let mut rows: std::collections::BTreeMap<usize, (String, String, Option<f64>)> = Default::default();
    for l in text.lines().map(str::trim) {
        let Some((k, v)) = l.split_once('=') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        let field = |name: &str| k.strip_prefix(name).and_then(|n| n.parse::<usize>().ok());
        if let Some(n) = field("file") {
            rows.entry(n).or_default().0 = v.to_owned();
        } else if let Some(n) = field("title") {
            rows.entry(n).or_default().1 = v.to_owned();
        } else if let Some(n) = field("length") {
            rows.entry(n).or_default().2 = v.parse::<f64>().ok().filter(|d| *d > 0.0);
        }
    }
    rows.into_values().filter(|(f, _, _)| !f.is_empty()).filter_map(|(f, t, d)| listed(&f, base, t, d)).collect()
}

/// MISC › Sort list, as Winamp offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Title,
    Filename,
    Path,
    Reverse,
    Random,
}

impl Sort {
    pub const ALL: [Sort; 5] = [Sort::Title, Sort::Filename, Sort::Path, Sort::Reverse, Sort::Random];

    pub fn id(self) -> &'static str {
        match self {
            Sort::Title => "sort:title",
            Sort::Filename => "sort:filename",
            Sort::Path => "sort:path",
            Sort::Reverse => "sort:reverse",
            Sort::Random => "sort:random",
        }
    }

    pub fn from_id(id: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Title => "Sort list by title",
            Sort::Filename => "Sort list by filename",
            Sort::Path => "Sort list by path and filename",
            Sort::Reverse => "Reverse list",
            Sort::Random => "Randomize list",
        }
    }
}

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn seed_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1
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
            let mut seed = seed_now();
            for i in (1..n).rev() {
                self.order.swap(i, (xorshift(&mut seed) % (i as u64 + 1)) as usize);
            }
        }
    }

    /// Reorders the list; the playing track keeps playing (its index moves).
    pub fn sort(&mut self, how: Sort) {
        let current = self.current().map(Entry::key);
        let name = |e: &Entry| match &e.url {
            Some(_) => e.display(),
            None => e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        match how {
            Sort::Title => self.entries.sort_by_cached_key(|e| e.display().to_lowercase()),
            Sort::Filename => self.entries.sort_by_cached_key(|e| name(e).to_lowercase()),
            Sort::Path => self.entries.sort_by_cached_key(|e| e.key().to_lowercase()),
            Sort::Reverse => self.entries.reverse(),
            Sort::Random => {
                let mut seed = seed_now();
                for i in (1..self.entries.len()).rev() {
                    self.entries.swap(i, (xorshift(&mut seed) % (i as u64 + 1)) as usize);
                }
            }
        }
        self.current = current.and_then(|p| self.entries.iter().position(|e| e.key() == p)).unwrap_or(0);
        self.reshuffle();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn song(path: &str, title: &str) -> Entry {
        Entry { path: PathBuf::from(path), title: title.into(), ..Default::default() }
    }

    #[test]
    fn sorts_keep_the_playing_track() {
        let mut p = Playlist::default();
        p.set(vec![song("/b/02 Zed.mp3", "Zed"), song("/a/09 Alpha.mp3", "Alpha"), Entry::remote("/media/x".into(), "Mid".into(), "Band".into(), None, false)]);
        p.current = 2;
        p.sort(Sort::Title);
        assert_eq!(p.entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Alpha", "Mid", "Zed"]);
        assert_eq!(p.current, 1, "Band - Mid still plays");
        p.sort(Sort::Filename);
        assert_eq!(p.entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Zed", "Alpha", "Mid"], "02 < 09 < Band");
        p.sort(Sort::Path);
        assert_eq!(p.entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Alpha", "Zed", "Mid"], "/a < /b < /media");
        p.sort(Sort::Reverse);
        assert_eq!(p.entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Mid", "Zed", "Alpha"]);
        assert_eq!(p.current, 0);
        p.sort(Sort::Random);
        assert_eq!(p.entries.len(), 3);
        assert_eq!(p.entries[p.current].title, "Mid");
        assert_eq!(Sort::from_id("sort:random"), Some(Sort::Random));
    }

    #[test]
    fn lists_files_and_links() {
        let dir = std::env::temp_dir().join(format!("gamp-lists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mp3"), b"").unwrap();
        let m3u = dir.join("mix.m3u8");
        std::fs::write(
            &m3u,
            "#EXTM3U\n#EXTINF:12,Local Song\na.mp3\n#EXTINF:-1,SomaFM\nhttp://ice.somafm.com/groovesalad\n#EXTINF:200,Art - Title\nhttp://127.0.0.1:8742/media/local/7?s=x\nnotes.txt\n",
        )
        .unwrap();
        let got = read_list(&m3u);
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], Listed::File(dir.join("a.mp3")));
        assert_eq!(got[1], Listed::Link { url: "http://ice.somafm.com/groovesalad".into(), title: "SomaFM".into(), duration: None });
        assert_eq!(got[2], Listed::Link { url: "http://127.0.0.1:8742/media/local/7?s=x".into(), title: "Art - Title".into(), duration: Some(200.0) });

        let pls = dir.join("mix.pls");
        std::fs::write(&pls, "[playlist]\nNumberOfEntries=2\nFile1=a.mp3\nTitle1=Song\nLength1=12\nFile2=https://x.example/live\nTitle2=Live One\nLength2=-1\n").unwrap();
        let got = read_list(&pls);
        assert_eq!(got, vec![Listed::File(dir.join("a.mp3")), Listed::Link { url: "https://x.example/live".into(), title: "Live One".into(), duration: None }]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Winamp's "Generate HTML playlist": a dark page listing the tracks.
pub fn html(entries: &[Entry]) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
    let total: f64 = entries.iter().filter_map(|e| e.duration).sum();
    let timed = entries.iter().filter(|e| e.duration.is_some()).count();
    let average = if timed > 0 { total / timed as f64 } else { 0.0 };
    let mut out = String::from(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>Gamp Generated PlayList</title>\n\
         <style>\nbody { background: #000080; color: #ffffff; font-family: Arial, Helvetica, sans-serif; margin: 0; padding: 20px; }\n\
         h1 { font-family: 'Arial Black', Arial, sans-serif; font-size: 40px; margin: 0; text-align: center; letter-spacing: 2px; }\n\
         h2 { font-size: 20px; font-weight: normal; margin: 0 0 20px; text-align: center; }\n\
         hr { border: 0; border-top: 1px solid #ffffff; }\n\
         p { font-size: 13px; }\n\
         ol { font-size: 14px; line-height: 1.6; }\n\
         .time { color: #a0a0ff; }\n\
         </style>\n</head>\n<body>\n<h1>GAMP</h1>\n<h2>playlist</h2>\n<hr>\n",
    );
    out.push_str(&format!(
        "<p>{} track{} in playlist, average track length: {}<br>\nPlaylist length: {}<br>\nRight-click here to save this HTML file.</p>\n<hr>\n<ol>\n",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" },
        clock_text(average),
        length_text(total),
    ));
    for e in entries {
        let name = if e.title.is_empty() { e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { e.display() };
        match e.duration {
            Some(d) => out.push_str(&format!("<li>{} <span class=\"time\">({})</span></li>\n", esc(&name), clock_text(d))),
            None => out.push_str(&format!("<li>{}</li>\n", esc(&name))),
        }
    }
    out.push_str("</ol>\n</body>\n</html>\n");
    out
}

fn clock_text(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn length_text(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    let (h, m, sec) = (s / 3600, s / 60 % 60, s % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h} hour{}", if h == 1 { "" } else { "s" }));
    }
    if m > 0 || h > 0 {
        parts.push(format!("{m} minute{}", if m == 1 { "" } else { "s" }));
    }
    parts.push(format!("{sec} second{}", if sec == 1 { "" } else { "s" }));
    parts.join(" ")
}

#[cfg(test)]
mod html_tests {
    use super::*;

    #[test]
    fn html_lists_tracks_and_totals() {
        let a = Entry { path: "/m/a.mp3".into(), title: "A <b>".into(), artist: "X".into(), duration: Some(90.0), ..Default::default() };
        let b = Entry::remote("/media/radio/1".into(), "Station".into(), String::new(), None, true);
        let page = html(&[a, b]);
        assert!(page.contains("2 tracks in playlist, average track length: 1:30"));
        assert!(page.contains("Playlist length: 1 minute 30 seconds"));
        assert!(page.contains("<li>X - A &lt;b&gt; <span class=\"time\">(1:30)</span></li>"));
        assert!(page.contains("<li>Station</li>"));
        assert_eq!(length_text(3661.0), "1 hour 1 minute 1 second");
    }
}
