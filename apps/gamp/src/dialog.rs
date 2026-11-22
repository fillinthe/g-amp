//! Dialogs: pick files or a folder, save a file, the update question.
//! On macOS they open just after the event that asked for them, from the
//! main queue: opened during it, Gamp stays mid-event for as long as the
//! dialog is up, and winit aborts if a window changes screens meanwhile
//! (a change of scale can't wait its turn). The answer comes back as an
//! event. Elsewhere they open at once.

use std::path::{Path, PathBuf};

use gamp_ui::lang::t;

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
    /// First start: about usage stats (yes: share them).
    Stats,
    /// Delete my data? (yes: delete).
    Forget,
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
            .set_title(t("Open files"))
            .add_filter(t("Audio and playlists"), &[playlist::AUDIO, &["m3u", "m3u8", "pls"]].concat())
            .add_filter(t("Winamp skins"), &["wsz", "zip"])
            .pick_files()
            .unwrap_or_default(),
        Ask::OpenFolder { .. } => one(FileDialog::new().set_title(t("Open a folder")).pick_folder()),
        Ask::PlaylistFile => one(FileDialog::new().add_filter(t("Playlists"), &["m3u", "m3u8", "pls"]).pick_file()),
        Ask::SkinFile => one(FileDialog::new().add_filter(t("Winamp skins"), &["wsz", "zip"]).pick_file()),
        Ask::PresetFile => one(FileDialog::new().add_filter(t("Winamp EQ presets"), &["eqf", "q1"]).pick_file()),
        Ask::MusicFolder => one(FileDialog::new().set_title(t("Add a music folder")).pick_folder()),
        Ask::SavePlaylist => save(t("Save playlist"), "playlists", t("M3U playlist"), "m3u8", &format!("{}.m3u8", t("My playlist"))),
        Ask::SavePreset => save(t("Save equalizer preset"), "presets", t("Winamp EQ preset"), "eqf", &format!("{}.eqf", t("My preset"))),
        Ask::Update(text) => {
            let answer = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title(t("A new Gamp is out"))
                .set_description(text)
                .set_buttons(rfd::MessageButtons::OkCancelCustom(t("Update now").into(), t("Later").into()))
                .show();
            yes = matches!(answer, rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes) || answer == rfd::MessageDialogResult::Custom(t("Update now").into());
            Vec::new()
        }
        Ask::Stats | Ask::Forget => {
            let (level, title, text, ok, no) = if matches!(ask, Ask::Stats) {
                (rfd::MessageLevel::Info, t("Anonymous usage stats"), format!("{}\n\n{}", t("To see what to improve, Gamp sends a few anonymous facts when it starts: its version, your system and language. Never your music, files, accounts or address."), t("Options › Share anonymous usage stats turns this off, and Options › Delete my data… erases everything.")), t("OK"), t("Don't share"))
            } else {
                (rfd::MessageLevel::Warning, t("Delete my data"), t("Gamp will forget everything: settings, sign-ins, playlists, skins and its anonymous usage stats, then close. This can't be undone.").to_string(), t("Delete"), t("Cancel"))
            };
            let answer = rfd::MessageDialog::new().set_level(level).set_title(title).set_description(&text).set_buttons(rfd::MessageButtons::OkCancelCustom(ok.into(), no.into())).show();
            yes = matches!(answer, rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes) || answer == rfd::MessageDialogResult::Custom(ok.into());
            Vec::new()
        }
    };
    Answer { ask, paths, yes }
}
