//! Jump to File (J): type part of a title, pick a match, Enter plays it.

use gamp_ui::jump_window::{self as jw, Row};
use gamp_ui::library_window::{self as lw, Hot};
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, Kind};

#[derive(Default)]
pub struct JumpState {
    pub view: jw::View,
    pub hots: Vec<(Hot, lw::R)>,
    /// Playlist indexes of the rows shown.
    pub matches: Vec<usize>,
    pub open: bool,
}

impl App {
    pub(crate) fn jump_toggle(&mut self) {
        self.jump_show(!self.jump.open);
    }

    pub(crate) fn jump_show(&mut self, on: bool) {
        self.jump.open = on;
        if on {
            // A fresh search each time, as Winamp's did.
            self.jump.view.search = lw::Field::new("Type part of a title");
            self.jump.view.search.focused = true;
            self.jump_filter();
            self.jump.view.focused = true;
        }
        if let Some(w) = self.wins.get_mut(&Kind::Jump) {
            w.show(on);
            if on {
                if let Some(window) = &w.window {
                    window.focus_window();
                }
            }
        }
        self.mark(Kind::Jump);
    }

    /// Rows for the entries whose "Artist - Title" has every typed word.
    pub(crate) fn jump_filter(&mut self) {
        let words: Vec<String> = self.jump.view.search.value.to_lowercase().split_whitespace().map(str::to_owned).collect();
        let current = (!self.playlist.entries.is_empty()).then_some(self.playlist.current);
        let keep = self.jump.view.selected.and_then(|s| self.jump.matches.get(s).copied());
        self.jump.matches = (0..self.playlist.entries.len())
            .filter(|i| {
                let hay = self.playlist.entries[*i].display().to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        self.jump.view.rows = self.jump.matches.iter().map(|i| Row { text: format!("{}. {}", i + 1, self.playlist.entries[*i].display()), current: current == Some(*i) }).collect();
        self.jump.view.status = gamp_ui::lang::tf("{} of {}", &[&self.jump.matches.len(), &self.playlist.entries.len()]);
        // Keep the pick if it's still listed; otherwise the first match
        // (the playing track when nothing is typed yet).
        self.jump.view.selected = match keep.and_then(|k| self.jump.matches.iter().position(|m| *m == k)) {
            Some(i) if !words.is_empty() || self.jump.view.selected.is_some() => Some(i),
            _ if words.is_empty() => current.and_then(|c| self.jump.matches.iter().position(|m| *m == c)).or((!self.jump.matches.is_empty()).then_some(0)),
            _ => (!self.jump.matches.is_empty()).then_some(0),
        };
        self.jump.view.top = self.jump.view.top.min(self.jump.view.max_top());
        if let Some(s) = self.jump.view.selected {
            self.jump.view.reveal(s);
        }
        self.mark(Kind::Jump);
    }

    fn jump_select(&mut self, d: isize) {
        let n = self.jump.matches.len();
        if n == 0 {
            return;
        }
        let at = self.jump.view.selected.map(|s| s as isize + d).unwrap_or(0).clamp(0, n as isize - 1) as usize;
        self.jump.view.selected = Some(at);
        self.jump.view.reveal(at);
        self.mark(Kind::Jump);
    }

    /// Plays the picked match and closes the window.
    fn jump_play(&mut self) {
        if let Some(i) = self.jump.view.selected.and_then(|s| self.jump.matches.get(s).copied()) {
            self.play_index(i);
            self.jump_show(false);
        }
    }

    pub(crate) fn jump_key(&mut self, key: &Key, text: Option<&str>) {
        let cmd = self.modifiers.control_key() || self.modifiers.super_key();
        let f = &mut self.jump.view.search;
        match key {
            Key::Named(NamedKey::Escape) => return self.jump_show(false),
            Key::Named(NamedKey::Enter) => return self.jump_play(),
            Key::Named(NamedKey::ArrowDown) => return self.jump_select(1),
            Key::Named(NamedKey::ArrowUp) => return self.jump_select(-1),
            Key::Named(NamedKey::PageDown) => return self.jump_select(self.jump.view.visible_rows() as isize),
            Key::Named(NamedKey::PageUp) => return self.jump_select(-(self.jump.view.visible_rows() as isize)),
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
            _ => match text.filter(|t| !cmd && !t.chars().any(char::is_control)) {
                Some(t) => f.insert(t),
                None => return,
            },
        }
        self.jump_filter();
    }

    fn jump_hot(&self, x: i32, y: i32) -> Option<Hot> {
        self.jump.hots.iter().rev().find(|(_, r)| lw::inside(*r, x, y)).map(|(h, _)| h.clone())
    }

    pub(crate) fn jump_press(&mut self, x: i32, y: i32) {
        let Some(h) = self.jump_hot(x, y) else {
            if y < 20 {
                self.start_drag(Kind::Jump);
            }
            return;
        };
        match &h {
            Hot::Row(i) => {
                let again = self.double_click(Kind::Jump, x, y) && self.jump.view.selected == Some(*i);
                self.jump.view.selected = Some(*i);
                if again {
                    self.jump_play();
                }
            }
            Hot::Close | Hot::Button(_) => self.jump.view.pressed = Some(h.clone()),
            _ => {}
        }
        self.mark(Kind::Jump);
    }

    pub(crate) fn jump_release(&mut self) {
        let Some(pressed) = self.jump.view.pressed.take() else { return };
        self.mark(Kind::Jump);
        let (x, y) = self.win(Kind::Jump).map(|w| w.cursor).unwrap_or((0, 0));
        if self.jump_hot(x, y) != Some(pressed.clone()) {
            return;
        }
        match pressed {
            Hot::Close => self.jump_show(false),
            Hot::Button(id) if id == "close" => self.jump_show(false),
            Hot::Button(id) if id == "play" => self.jump_play(),
            _ => {}
        }
    }

    pub(crate) fn jump_wheel(&mut self, lines: f64) {
        let top = self.jump.view.top as isize - (lines * 3.0).round() as isize;
        self.jump.view.top = top.clamp(0, self.jump.view.max_top() as isize) as usize;
        self.mark(Kind::Jump);
    }

    pub(crate) fn jump_redraw(&mut self) {
        if let Some(w) = self.wins.get_mut(&Kind::Jump) {
            self.jump.hots = jw::draw(&mut w.frame, &self.skin, &self.jump.view);
            w.present();
        }
    }
}
