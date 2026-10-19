//! The local library: every audio file under the configured folders,
//! including network shares (mounted under /Volumes on macOS, mapped drives
//! or \\server\share paths on Windows). Tags are read once and kept in
//! library.json; a rescan re-reads only files whose size or time changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use lofty::prelude::*;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::store;

pub const AUDIO_TYPES: &[(&str, &str)] = &[
    ("mp3", "audio/mpeg"),
    ("m4a", "audio/mp4"),
    ("aac", "audio/aac"),
    ("mp4", "audio/mp4"),
    ("flac", "audio/flac"),
    ("wav", "audio/wav"),
    ("aif", "audio/aiff"),
    ("aiff", "audio/aiff"),
    ("ogg", "audio/ogg"),
    ("oga", "audio/ogg"),
    ("opus", "audio/ogg"),
    ("webm", "audio/webm"),
];

/// Folders that NAS boxes and operating systems fill with things that are
/// not music (Synology's thumbnails and recycle bin among them).
const SKIP_DIRS: &[&str] = &[
    "@eaDir",
    "#recycle",
    "#snapshot",
    ".AppleDouble",
    ".Trashes",
    ".Spotlight-V100",
    ".fseventsd",
    "$RECYCLE.BIN",
    "System Volume Information",
    "node_modules",
];

pub fn mime_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    AUDIO_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m).unwrap_or("application/octet-stream")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime: f64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: String,
    pub duration: Option<u32>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub scanning: bool,
    pub scanned: u64,
    pub found: u64,
    pub errors: u64,
    pub finished_at: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    finished_at: Option<u64>,
    tracks: Vec<Track>,
}

struct State {
    tracks: HashMap<String, Track>,
    status: Status,
}

static STATE: OnceLock<Arc<Mutex<State>>> = OnceLock::new();

async fn state() -> Arc<Mutex<State>> {
    if let Some(s) = STATE.get() {
        return s.clone();
    }
    let saved = store::read_json::<Saved>("library.json").await.unwrap_or_default();
    let status = Status { found: saved.tracks.len() as u64, finished_at: saved.finished_at, ..Default::default() };
    let tracks = saved.tracks.into_iter().map(|t| (t.id.clone(), t)).collect();
    let _ = STATE.set(Arc::new(Mutex::new(State { tracks, status })));
    STATE.get().unwrap().clone()
}

pub fn id_for(path: &Path) -> String {
    let digest = Sha1::digest(path.to_string_lossy().as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub async fn list() -> Vec<Track> {
    state().await.lock().unwrap().tracks.values().cloned().collect()
}

pub async fn get(id: &str) -> Option<Track> {
    state().await.lock().unwrap().tracks.get(id).cloned()
}

pub async fn status() -> Status {
    state().await.lock().unwrap().status.clone()
}

async fn persist(state: &Arc<Mutex<State>>) {
    let saved = {
        let s = state.lock().unwrap();
        Saved { finished_at: s.status.finished_at.clone(), tracks: s.tracks.values().cloned().collect() }
    };
    if let Err(e) = store::write_json("library.json", &saved, false).await {
        log::warn!("could not save the library: {e}");
    }
}

pub async fn rescan() -> Status {
    let st = state().await;
    {
        let mut s = st.lock().unwrap();
        if s.status.scanning {
            return s.status.clone();
        }
        s.status.scanning = true;
        s.status.scanned = 0;
        s.status.errors = 0;
    }
    let folders = store::config().await.folders;
    let st2 = st.clone();
    tokio::spawn(async move {
        let st3 = st2.clone();
        let result = tokio::task::spawn_blocking(move || scan(&st3, &folders)).await;
        if let Err(e) = result {
            log::error!("scan failed: {e}");
        }
        {
            let mut s = st2.lock().unwrap();
            s.status.scanning = false;
            s.status.found = s.tracks.len() as u64;
            s.status.finished_at = Some(now_ms());
        }
        persist(&st2).await;
    });
    st.lock().unwrap().status.clone()
}

/// Milliseconds since the epoch, as the page's Date() takes them.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn is_audio(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    AUDIO_TYPES.iter().any(|(e, _)| *e == ext)
}

fn read_track(path: &Path, size: u64, mtime: f64) -> (Track, bool) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut t = Track {
        id: id_for(path),
        path: path.to_path_buf(),
        size,
        mtime,
        title: stem,
        artist: String::new(),
        album: String::new(),
        track_no: None,
        year: None,
        genre: String::new(),
        duration: None,
    };
    match lofty::read_from_path(path) {
        Ok(file) => {
            let secs = file.properties().duration().as_secs_f64();
            if secs > 0.0 {
                t.duration = Some(secs.round() as u32);
            }
            if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
                if let Some(v) = tag.title().filter(|v| !v.trim().is_empty()) {
                    t.title = v.into_owned();
                }
                t.artist = tag.artist().map(|v| v.into_owned()).unwrap_or_default();
                if t.artist.is_empty() {
                    t.artist = tag.get_string(&lofty::tag::ItemKey::AlbumArtist).unwrap_or_default().to_owned();
                }
                t.album = tag.album().map(|v| v.into_owned()).unwrap_or_default();
                t.genre = tag.genre().map(|v| v.into_owned()).unwrap_or_default();
                t.track_no = tag.track();
                t.year = tag.year();
            }
            (t, true)
        }
        Err(_) => (t, false),
    }
}

fn scan(state: &Arc<Mutex<State>>, folders: &[PathBuf]) {
    let mut seen = HashSet::new();
    for folder in folders {
        let walker = walkdir::WalkDir::new(folder).follow_links(true).into_iter().filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !(name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()))
        });
        for entry in walker {
            let Ok(entry) = entry else {
                state.lock().unwrap().status.errors += 1;
                continue;
            };
            if !entry.file_type().is_file() || !is_audio(entry.path()) {
                continue;
            }
            let path = entry.path();
            let id = id_for(path);
            seen.insert(id.clone());
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            let unchanged = {
                let s = state.lock().unwrap();
                s.tracks.get(&id).is_some_and(|t| t.size == meta.len() && t.mtime == mtime)
            };
            let mut s = if unchanged {
                state.lock().unwrap()
            } else {
                let (track, ok) = read_track(path, meta.len(), mtime);
                let mut s = state.lock().unwrap();
                if !ok {
                    s.status.errors += 1;
                }
                s.tracks.insert(id, track);
                s
            };
            s.status.scanned += 1;
            s.status.found = s.tracks.len() as u64;
        }
    }
    // Forget files that are gone, but only under folders that answered: a
    // share that is not mounted right now must not empty the library.
    let reachable: Vec<&PathBuf> = folders.iter().filter(|f| f.exists()).collect();
    let mut s = state.lock().unwrap();
    s.tracks.retain(|id, t| {
        let configured = folders.iter().any(|f| t.path.starts_with(f));
        let under_reachable = reachable.iter().any(|f| t.path.starts_with(f));
        configured && (!under_reachable || seen.contains(id))
    });
}
