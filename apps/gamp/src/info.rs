//! File Info (Alt+3, MISC › File info): a track's tags, which you can
//! edit and save for files, with its format, size, length and place.

use std::path::PathBuf;

use gamp_ui::info_window as iw;
use gamp_ui::library_window::{self as lw, Field, Hot};
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, Kind, clock};
use crate::playlist::Entry;

/// The tags you can edit, in order.
const TAGS: [&str; 7] = ["Title", "Artist", "Album", "Year", "Track", "Genre", "Comment"];

#[derive(Default)]
pub struct InfoState {
    pub view: iw::View,
    pub hots: Vec<(Hot, lw::R)>,
    pub open: bool,
    /// The playlist entry shown, and its file when the tags can be saved.
    pub index: Option<usize>,
    pub path: Option<PathBuf>,
    /// The field being typed in.
    pub focus: Option<usize>,
}

/// The editable tags, format rows and length for a file, from the file.
fn file_info(e: &Entry) -> (Vec<String>, Vec<(String, String)>, Option<f64>) {
    use lofty::prelude::*;
    let mut tags = vec![String::new(); TAGS.len()];
    let mut rows = Vec::new();
    let mut duration = e.duration;
    let Ok(file) = lofty::read_from_path(&e.path) else {
        tags[0] = e.title.clone();
        tags[1] = e.artist.clone();
        return (tags, rows, duration);
    };
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        let s = |v: Option<std::borrow::Cow<str>>| v.map(|v| v.trim().to_owned()).unwrap_or_default();
        tags[0] = s(tag.title());
        tags[1] = s(tag.artist());
        tags[2] = s(tag.album());
        tags[3] = tag.year().map(|y| y.to_string()).unwrap_or_default();
        tags[4] = match (tag.track(), tag.track_total()) {
            (Some(t), Some(n)) => format!("{t}/{n}"),
            (Some(t), None) => t.to_string(),
            _ => String::new(),
        };
        tags[5] = s(tag.genre());
        tags[6] = s(tag.comment());
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
            1 => gamp_ui::lang::t("mono").into(),
            2 => gamp_ui::lang::t("stereo").into(),
            n => gamp_ui::lang::tf("{} channels", &[&n]),
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
    (tags, rows, duration)
}

/// Writes the tags to the file; empty fields are removed.
fn save_tags(path: &std::path::Path, tags: &[String]) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::prelude::*;
    use lofty::tag::Tag;
    let mut file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    if file.primary_tag_mut().is_none() {
        let t = file.primary_tag_type();
        file.insert_tag(Tag::new(t));
    }
    let tag = file.primary_tag_mut().ok_or("no tag")?;
    let text = |i: usize| tags.get(i).map(|t| t.trim()).filter(|t| !t.is_empty()).map(str::to_owned);
    match text(0) {
        Some(v) => tag.set_title(v),
        None => tag.remove_title(),
    }
    match text(1) {
        Some(v) => tag.set_artist(v),
        None => tag.remove_artist(),
    }
    match text(2) {
        Some(v) => tag.set_album(v),
        None => tag.remove_album(),
    }
    match text(3).and_then(|v| v.parse::<u32>().ok()) {
        Some(v) => tag.set_year(v),
        None => tag.remove_year(),
    }
    let (track, total) = match text(4) {
        Some(v) => {
            let (a, b) = v.split_once(['/', ' ']).map(|(a, b)| (a.trim(), b.trim().trim_start_matches("of").trim())).unwrap_or((v.as_str(), ""));
            (a.parse::<u32>().ok(), b.parse::<u32>().ok())
        }
        None => (None, None),
    };
    match track {
        Some(v) => tag.set_track(v),
        None => tag.remove_track(),
    }
    match total {
        Some(v) => tag.set_track_total(v),
        None => tag.remove_track_total(),
    }
    match text(5) {
        Some(v) => tag.set_genre(v),
        None => tag.remove_genre(),
    }
    match text(6) {
        Some(v) => tag.set_comment(v),
        None => tag.remove_comment(),
    }
    file.save_to_path(path, WriteOptions::default()).map_err(|e| e.to_string())
}

impl App {
    /// Shows the box for the first selected track, else the playing one.
    pub(crate) fn info_show_selected(&mut self) {
        let i = self.pl.selected.iter().next().copied().unwrap_or(self.playlist.current);
        self.info_show(Some(i));
    }

    pub(crate) fn info_show(&mut self, index: Option<usize>) {
        let Some(e) = index.and_then(|i| self.playlist.entries.get(i)).cloned() else {
            self.say("NOTHING TO SHOW");
            self.message_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
            return;
        };
        let n = index.map(|i| i + 1).unwrap_or(0);
        let v = &mut self.info.view;
        v.fields.clear();
        v.status.clear();
        self.info.index = index;
        self.info.path = None;
        self.info.focus = None;
        let (mut rows, duration, place) = match &e.url {
            None => {
                let (tags, rows, d) = file_info(&e);
                v.fields = TAGS.iter().zip(tags).map(|(l, t)| (l.to_string(), Field { value: t, ..Default::default() })).collect();
                self.info.path = Some(e.path.clone());
                (rows, d, e.path.display().to_string())
            }
            Some(u) => {
                let mut rows = vec![("Title".to_owned(), e.title.clone())];
                if !e.artist.is_empty() {
                    rows.push(("Artist".to_owned(), e.artist.clone()));
                }
                rows.push(("Source".to_owned(), gamp_ui::lang::t(if e.live { "Internet stream" } else { "Network track" }).to_owned()));
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
        let name = match &e.url {
            None => e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Some(_) => e.display(),
        };
        let v = &mut self.info.view;
        v.heading = format!("{n}. {name}");
        v.rows = rows;
        v.place = place;
        v.focused = true;
        self.info.open = true;
        if !self.info.view.fields.is_empty() {
            self.info_focus(Some(0));
        }
        // Tall enough for everything, wrapped.
        self.resize_window(Kind::Info);
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

    fn info_focus(&mut self, i: Option<usize>) {
        self.info.focus = i;
        for (n, (_, f)) in self.info.view.fields.iter_mut().enumerate() {
            f.focused = Some(n) == i;
            if f.focused {
                f.end();
            }
        }
        self.mark(Kind::Info);
    }

    /// Writes the fields to the file and shows the new tags in the list.
    pub(crate) fn info_save(&mut self) {
        let Some(path) = self.info.path.clone() else { return };
        let tags: Vec<String> = self.info.view.fields.iter().map(|(_, f)| f.value.clone()).collect();
        match save_tags(&path, &tags) {
            Ok(()) => {
                if let Some(e) = self.info.index.and_then(|i| self.playlist.entries.get_mut(i)) {
                    let fresh = Entry::read(&path);
                    e.title = fresh.title;
                    e.artist = fresh.artist;
                }
                self.info.view.status = "Saved".into();
                self.update_title();
            }
            Err(e) => self.info.view.status = gamp_ui::lang::tf("Can't save: {}", &[&e]),
        }
        self.mark(Kind::Info);
    }

    /// Everything in the box, to the clipboard.
    pub(crate) fn info_copy(&mut self) {
        crate::clipboard::set(&self.info.view.text());
        self.info.view.status = "Copied".into();
        self.mark(Kind::Info);
    }

    pub(crate) fn info_key(&mut self, key: &Key, text: Option<&str>) {
        let cmd = self.modifiers.control_key() || self.modifiers.super_key();
        if cmd && matches!(key, Key::Character(c) if c.as_str() == "c") {
            return self.info_copy();
        }
        let n = self.info.view.fields.len();
        match key {
            Key::Named(NamedKey::Escape) => return self.info_hide(),
            Key::Named(NamedKey::Enter) => {
                if self.info.path.is_some() {
                    self.info_save();
                } else {
                    self.info_hide();
                }
                return;
            }
            Key::Named(NamedKey::Tab) if n > 0 => {
                let back = self.modifiers.shift_key();
                let next = match self.info.focus {
                    Some(i) if back => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                    None => 0,
                };
                return self.info_focus(Some(next));
            }
            Key::Named(NamedKey::ArrowDown) if n > 0 => return self.info_focus(Some(self.info.focus.map(|i| (i + 1) % n).unwrap_or(0))),
            Key::Named(NamedKey::ArrowUp) if n > 0 => return self.info_focus(Some(self.info.focus.map(|i| (i + n - 1) % n).unwrap_or(0))),
            _ => {}
        }
        let Some(f) = self.info.focus.and_then(|i| self.info.view.fields.get_mut(i)).map(|(_, f)| f) else { return };
        match key {
            Key::Named(NamedKey::Backspace) => f.backspace(),
            Key::Named(NamedKey::Delete) => f.delete(),
            Key::Named(NamedKey::ArrowLeft) => f.move_caret(-1),
            Key::Named(NamedKey::ArrowRight) => f.move_caret(1),
            Key::Named(NamedKey::Home) => f.caret = 0,
            Key::Named(NamedKey::End) => f.end(),
            Key::Character(c) if cmd && c.as_str() == "v" => {
                if let Some(t) = crate::clipboard::get() {
                    f.insert(t.lines().next().unwrap_or(""));
                }
            }
            Key::Character(c) if cmd && c.as_str() == "a" => f.end(),
            _ => {
                if let Some(t) = text.filter(|t| !cmd && !t.chars().any(char::is_control)) {
                    f.insert(t);
                }
            }
        }
        self.info.view.status.clear();
        self.mark(Kind::Info);
    }

    fn info_hot(&self, x: i32, y: i32) -> Option<Hot> {
        self.info.hots.iter().rev().find(|(_, r)| lw::inside(*r, x, y)).map(|(h, _)| h.clone())
    }

    pub(crate) fn info_press(&mut self, x: i32, y: i32) {
        match self.info_hot(x, y) {
            Some(h @ (Hot::Close | Hot::Button(_))) => self.info.view.pressed = Some(h),
            Some(Hot::Form(label)) => {
                let i = self.info.view.fields.iter().position(|(l, _)| *l == label);
                self.info_focus(i);
            }
            None if y < 20 => self.start_drag(Kind::Info),
            _ => {}
        }
        self.mark(Kind::Info);
    }

    pub(crate) fn info_release(&mut self) {
        let Some(pressed) = self.info.view.pressed.take() else { return };
        self.mark(Kind::Info);
        let (x, y) = self.win(Kind::Info).map(|w| w.cursor).unwrap_or((0, 0));
        if self.info_hot(x, y) != Some(pressed.clone()) {
            return;
        }
        match pressed {
            Hot::Button(id) if id == "save" => self.info_save(),
            Hot::Button(id) if id == "copy" => self.info_copy(),
            _ => self.info_hide(),
        }
    }

    pub(crate) fn info_redraw(&mut self) {
        if let Some(w) = self.wins.get_mut(&Kind::Info) {
            self.info.hots = iw::draw(&mut w.frame, &self.skin, &self.info.view);
            w.present();
        }
    }
}
