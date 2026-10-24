//! Dialogs: pick files or a folder, save a file, the update question.
//! On macOS they open just after the event that asked for them, from the
//! main queue: opened during it, Gamp stays mid-event for as long as the
//! dialog is up, and winit aborts if a window changes screens meanwhile
//! (a change of scale can't wait its turn). The answer comes back as an
//! event. Elsewhere they open at once.

use std::path::{Path, PathBuf};

use crate::playlist;

#[derive(Clone, Debug)]
pub enum Ask {
    OpenFiles { replace: bool },
    OpenFolder { replace: bool },
    PlaylistFile,
    SkinFile,
    PresetFile,
    MusicFolder,
    SavePlaylist,
    SavePreset,
    /// "Gamp X is out: update now?", with what's new.
    Update(String),
}

#[derive(Debug)]
pub struct Answer {
    pub ask: Ask,
    /// What was picked (empty when cancelled).
    pub paths: Vec<PathBuf>,
    /// Update now (the update question).
    pub yes: bool,
}

/// Shows the dialog and waits for it. `data`: Gamp's folder, where saved
/// playlists and presets go.
pub fn run(ask: Ask, data: &Path) -> Answer {
    use rfd::FileDialog;
    let one = |p: Option<PathBuf>| p.into_iter().collect::<Vec<_>>();
    let save = |title: &str, sub: &str, kind: &str, ext: &str, name: &str| {
        let dir = data.join(sub);
        let _ = std::fs::create_dir_all(&dir);
        one(FileDialog::new().set_title(title).set_directory(&dir).add_filter(kind, &[ext]).set_file_name(name).save_file())
    };
    let mut yes = false;
    let paths = match &ask {
        Ask::OpenFiles { .. } => FileDialog::new()
            .set_title("Open files")
            .add_filter("Audio and playlists", &[playlist::AUDIO, &["m3u", "m3u8", "pls"]].concat())
            .add_filter("Winamp skins", &["wsz", "zip"])
            .pick_files()
            .unwrap_or_default(),
        Ask::OpenFolder { .. } => one(FileDialog::new().set_title("Open a folder").pick_folder()),
        Ask::PlaylistFile => one(FileDialog::new().add_filter("Playlists", &["m3u", "m3u8", "pls"]).pick_file()),
        Ask::SkinFile => one(FileDialog::new().add_filter("Winamp skins", &["wsz", "zip"]).pick_file()),
        Ask::PresetFile => one(FileDialog::new().add_filter("Winamp EQ presets", &["eqf", "q1"]).pick_file()),
        Ask::MusicFolder => one(FileDialog::new().set_title("Add a music folder").pick_folder()),
        Ask::SavePlaylist => save("Save playlist", "playlists", "M3U playlist", "m3u8", "My playlist.m3u8"),
        Ask::SavePreset => save("Save equalizer preset", "presets", "Winamp EQ preset", "eqf", "My preset.eqf"),
        Ask::Update(text) => {
            let answer = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title("A new Gamp is out")
                .set_description(text)
                .set_buttons(rfd::MessageButtons::OkCancelCustom("Update now".into(), "Later".into()))
                .show();
            yes = matches!(answer, rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes) || answer == rfd::MessageDialogResult::Custom("Update now".into());
            Vec::new()
        }
    };
    Answer { ask, paths, yes }
}
