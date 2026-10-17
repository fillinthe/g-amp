//! File Info (Alt+3, MISC › File info): what's known about a track.

use gamp_ui::info_window as iw;
use gamp_ui::library_window::{self as lw, Hot};
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, Kind, clock};
use crate::playlist::Entry;

#[derive(Default)]
pub struct InfoState {
    pub view: iw::View,
    pub hots: Vec<(Hot, lw::R)>,
    pub open: bool,
}

/// Tags and format for a file, read from the file itself.
fn file_rows(e: &Entry) -> (Vec<(String, String)>, Option<f64>) {
    use lofty::prelude::*;
    let mut rows = Vec::new();
    let mut duration = e.duration;
    let Ok(file) = lofty::read_from_path(&e.path) else { return (rows, duration) };
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        let s = |v: Option<std::borrow::Cow<str>>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        for (label, value) in [
            ("Artist", s(tag.artist())),
            ("Album", s(tag.album())),
            ("Year", tag.year().map(|y| y.to_string())),
            ("Track", tag.track().map(|t| match tag.track_total() {
                Some(n) => format!("{t} of {n}"),
                None => t.to_string(),
            })),
            ("Genre", s(tag.genre())),
            ("Comment", s(tag.comment())),
        ] {
            if let Some(v) = value {
                rows.push((label.to_owned(), v));
            }
        }
    }
    let p = file.properties();
    let d = p.duration().as_secs_f64();
    if d > 0.0 {
        duration = Some(d);
    }
    let kind = match file.file_type() {
        lofty::file::FileType::Mpeg => "MP3".to_owned(),
        lofty::file::FileType::Mp4 => "MP4/AAC".to_owned(),
        other => format!("{other:?}").to_uppercase(),
    };
    let mut format = vec![kind];
    if let Some(b) = p.audio_bitrate() {
        format.push(format!("{b} kbps"));
    }
    if let Some(r) = p.sample_rate() {
        format.push(format!("{} kHz", r as f32 / 1000.0));
    }
    if let Some(c) = p.channels() {
        format.push(match c {
            1 => "mono".into(),
            2 => "stereo".into(),
            n => format!("{n} channels"),
        });
    }
    if let Some(b) = p.bit_depth() {
        format.push(format!("{b}-bit"));
    }
    rows.push(("Format".to_owned(), format.join(", ")));
    if let Ok(m) = std::fs::metadata(&e.path) {
        let bytes = m.len();
        let size = if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64) } else { format!("{} KB", bytes / 1024) };
        rows.push(("Size".to_owned(), size));
    }
    (rows, duration)
}

impl App {
    /// Shows the box for the first selected track, else the playing one.
    pub(crate) fn info_show_selected(&mut self) {
        let i = self.pl.selected.iter().next().copied().unwrap_or(self.playlist.current);
        self.info_show(Some(i));
    }

    pub(crate) fn info_show(&mut self, index: Option<usize>) {
        let Some(e) = index.and_then(|i| self.playlist.entries.get(i)).cloned() else {
            self.main.message = Some("NOTHING TO SHOW".into());
            self.message_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
            return;
        };
        let (mut rows, duration, place) = match &e.url {
            None => {
                let (rows, d) = file_rows(&e);
                (rows, d, e.path.display().to_string())
            }
            Some(u) => {
                let mut rows = Vec::new();
                if !e.artist.is_empty() {
                    rows.push(("Artist".to_owned(), e.artist.clone()));
                }
                rows.push(("Source".to_owned(), if e.live { "Internet stream".to_owned() } else { "Network track".to_owned() }));
                // What's playing right now tells us its format.
                if index == Some(self.playlist.current) && self.main.state != gamp_ui::main_window::PlayState::Stopped {
                    let mut f = Vec::new();
                    if let Some(k) = self.main.kbps.filter(|k| *k > 0) {
                        f.push(format!("{k} kbps"));
                    }
                    if let Some(k) = self.main.khz {
                        f.push(format!("{k} kHz"));
                    }
                    if let Some(st) = self.main.stereo {
                        f.push(if st { "stereo".to_owned() } else { "mono".to_owned() });
                    }
                    if !f.is_empty() {
                        rows.push(("Format".to_owned(), f.join(", ")));
                    }
                }
                (rows, e.duration, self.backend.as_ref().map(|b| b.url(u)).unwrap_or_else(|| u.clone()))
            }
        };
        if let Some(d) = duration.filter(|_| !e.live) {
            rows.push(("Length".to_owned(), clock(d)));
        }
        let n = index.map(|i| i + 1).unwrap_or(0);
        rows.insert(0, (String::new(), format!("{n}. {}", if e.title.is_empty() { e.display() } else { e.title.clone() })));
        self.info.view.rows = rows;
        self.info.view.place = place;
        self.info.view.focused = true;
        self.info.open = true;
        if let Some(w) = self.wins.get_mut(&Kind::Info) {
            w.show(true);
            if let Some(window) = &w.window {
                window.focus_window();
            }
        }
        self.mark(Kind::Info);
    }

    pub(crate) fn info_hide(&mut self) {
        self.info.open = false;
        if let Some(w) = self.wins.get_mut(&Kind::Info) {
            w.show(false);
        }
    }

    pub(crate) fn info_key(&mut self, key: &Key) {
        if matches!(key, Key::Named(NamedKey::Escape | NamedKey::Enter)) {
            self.info_hide();
        }
    }

    fn info_hot(&self, x: i32, y: i32) -> Option<Hot> {
        self.info.hots.iter().rev().find(|(_, r)| lw::inside(*r, x, y)).map(|(h, _)| h.clone())
    }

    pub(crate) fn info_press(&mut self, x: i32, y: i32) {
        match self.info_hot(x, y) {
            Some(h @ (Hot::Close | Hot::Button(_))) => self.info.view.pressed = Some(h),
            None if y < 20 => self.start_drag(Kind::Info),
            _ => {}
        }
        self.mark(Kind::Info);
    }

    pub(crate) fn info_release(&mut self) {
        let Some(pressed) = self.info.view.pressed.take() else { return };
        self.mark(Kind::Info);
        let (x, y) = self.win(Kind::Info).map(|w| w.cursor).unwrap_or((0, 0));
        if self.info_hot(x, y) == Some(pressed) {
            self.info_hide();
        }
    }

    pub(crate) fn info_redraw(&mut self) {
        if let Some(w) = self.wins.get_mut(&Kind::Info) {
            self.info.hots = iw::draw(&mut w.frame, &self.skin, &self.info.view);
            w.present();
        }
    }
}
