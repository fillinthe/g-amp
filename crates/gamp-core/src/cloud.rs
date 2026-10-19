//! Cloud drives through their own desktop apps: Dropbox, Google Drive and
//! OneDrive each keep a folder on this computer, signed in with your
//! account. Gamp lists that folder by name, without reading tags (that
//! would download every online-only file); a file downloads when played.

use std::path::{Component, Path, PathBuf};

use axum::http::StatusCode;

use crate::http::{ApiError, err};
use crate::sources::{Item, Listing, is_audio, sign};

/// A cloud folder found on this computer.
#[derive(Clone, Debug)]
pub struct Folder {
    /// The source id, e.g. "cloud-dropbox".
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// "Dropbox", "GoogleDrive-you@gmail.com", "OneDrive-Personal" → a name to show.
fn name_for(dir: &str) -> Option<String> {
    let (kind, rest) = dir.split_once('-').unwrap_or((dir, ""));
    let label = match kind {
        "Dropbox" => "Dropbox",
        "GoogleDrive" => "Google Drive",
        "OneDrive" => "OneDrive",
        _ => return None,
    };
    Some(if rest.is_empty() || rest == "Personal" { label.to_owned() } else { format!("{label} ({rest})") })
}

/// Cloud folders on this computer: macOS keeps them in
/// ~/Library/CloudStorage; Dropbox and OneDrive also use ~/Dropbox and
/// ~/OneDrive (Windows, Linux, older macOS).
pub fn folders() -> Vec<Folder> {
    let home = home();
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(home.join("Library/CloudStorage")) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(label) = name_for(&name) {
                found.push((label, e.path()));
            }
        }
    }
    for (label, path) in [("Dropbox", home.join("Dropbox")), ("OneDrive", std::env::var_os("OneDrive").map(PathBuf::from).unwrap_or_else(|| home.join("OneDrive")))] {
        if !found.iter().any(|(l, _)| l == label) && path.is_dir() {
            found.push((label.to_owned(), path));
        }
    }
    found.sort();
    found
        .into_iter()
        .map(|(name, path)| {
            let id = format!("cloud-{}", name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>());
            Folder { id, name, path }
        })
        .collect()
}

fn find(sid: &str) -> Result<Folder, ApiError> {
    folders().into_iter().find(|f| f.id == sid).ok_or_else(|| err(StatusCode::NOT_FOUND, "That cloud folder isn't on this computer any more"))
}

/// `rel` inside `root`, refusing anything that climbs out of it.
fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    rel.components().all(|c| matches!(c, Component::Normal(_))).then(|| root.join(rel))
}

fn stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_owned()
}

/// A folder's subfolders and audio files. `node` is "files" for the top,
/// or "dir:<path inside>".
pub fn browse(sid: &str, node: &str) -> Result<Listing, ApiError> {
    let folder = find(sid)?;
    let rel = node.strip_prefix("dir:").unwrap_or("");
    let dir = inside(&folder.path, rel).ok_or_else(|| err(StatusCode::BAD_REQUEST, "Not a folder in there"))?;
    let entries = std::fs::read_dir(&dir).map_err(|e| err(StatusCode::NOT_FOUND, format!("Can't open {}: {e}", dir.display())))?;
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            dirs.push(Item::folder(format!("dir:{path}"), name));
        } else if is_audio(&name) {
            let mut i = Item::track(path.clone(), stem(&name));
            i.url = Some(format!("/media/cloud/{}?p={}&s={}", sid, urlencoding::encode(&path), sign(sid, &path)));
            files.push(i);
        }
    }
    dirs.sort_by_key(|i| i.title.to_lowercase());
    files.sort_by_key(|i| i.title.to_lowercase());
    dirs.append(&mut files);
    let empty = dirs.is_empty();
    let l = Listing::of(dirs);
    Ok(if empty { l.hint("No folders or music here.") } else { l })
}

/// The file behind a listed track, if `sig` shows Gamp listed it.
pub fn file(sid: &str, rel: &str, sig: &str) -> Result<PathBuf, ApiError> {
    if sign(sid, rel) != sig {
        return Err(err(StatusCode::FORBIDDEN, "Not a track this app listed"));
    }
    inside(&find(sid)?.path, rel).ok_or_else(|| err(StatusCode::BAD_REQUEST, "Not a file in there"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(name_for("Dropbox").as_deref(), Some("Dropbox"));
        assert_eq!(name_for("OneDrive-Personal").as_deref(), Some("OneDrive"));
        assert_eq!(name_for("GoogleDrive-you@gmail.com").as_deref(), Some("Google Drive (you@gmail.com)"));
        assert_eq!(name_for("iCloud Drive"), None);
        let root = Path::new("/r");
        assert_eq!(inside(root, "a/b.mp3"), Some(PathBuf::from("/r/a/b.mp3")));
        assert_eq!(inside(root, "../etc/passwd"), None);
        assert_eq!(inside(root, "/etc/passwd"), None);
        assert_eq!(inside(root, "a/../../x"), None);
    }
}
