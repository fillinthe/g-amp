//! The playlist editor: resizable in Winamp's 25×29 steps, tracks in the
//! skin's PLEDIT colours, the ADD/REM/SEL/MISC/LIST pop-up menus, a mini
//! transport and the running time.

use std::collections::BTreeSet;

use gamp_skin::Skin;

use crate::font;
use crate::frame::{Frame, argb};

pub const MIN_WIDTH: u32 = 275;
pub const MIN_HEIGHT: u32 = 116;
pub const SHADE_HEIGHT: u32 = 14;
pub const ROW: i32 = 13;
const FONT_PX: f32 = 9.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Add,
    Remove,
    Select,
    Misc,
    List,
}

impl Menu {
    /// Items from top to bottom, as sprite base names.
    pub fn items(self) -> &'static [&'static str] {
        match self {
            Menu::Add => &["PLAYLIST_ADD_URL", "PLAYLIST_ADD_DIR", "PLAYLIST_ADD_FILE"],
            Menu::Remove => &["PLAYLIST_REMOVE_ALL", "PLAYLIST_CROP", "PLAYLIST_REMOVE_SELECTED", "PLAYLIST_REMOVE_MISC"],
            Menu::Select => &["PLAYLIST_INVERT_SELECTION", "PLAYLIST_SELECT_ZERO", "PLAYLIST_SELECT_ALL"],
            Menu::Misc => &["PLAYLIST_SORT_LIST", "PLAYLIST_FILE_INFO", "PLAYLIST_MISC_OPTIONS"],
            Menu::List => &["PLAYLIST_NEW_LIST", "PLAYLIST_SAVE_LIST", "PLAYLIST_LOAD_LIST"],
        }
    }

    fn bar(self) -> &'static str {
        match self {
            Menu::Add => "PLAYLIST_ADD_MENU_BAR",
            Menu::Remove => "PLAYLIST_REMOVE_MENU_BAR",
            Menu::Select => "PLAYLIST_SELECT_MENU_BAR",
            Menu::Misc => "PLAYLIST_MISC_MENU_BAR",
            Menu::List => "PLAYLIST_LIST_BAR",
        }
    }

    /// The button's left edge in a window `width` wide.
    pub fn x(self, width: i32) -> i32 {
        match self {
            Menu::Add => 14,
            Menu::Remove => 43,
            Menu::Select => 72,
            Menu::Misc => 101,
            Menu::List => width - 44,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mini {
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    TitleBar,
    Close,
    Shade,
    /// A row, by index into the whole list.
    Track(usize),
    /// Below the last track.
    Empty,
    Scroll,
    Menu(Menu),
    Mini(Mini),
    Resize,
}

#[derive(Clone, Default)]
pub struct Track {
    pub title: String,
    /// "3:25", or empty.
    pub length: String,
}

#[derive(Clone, Default)]
pub struct View {
    pub width: u32,
    pub height: u32,
    pub tracks: Vec<Track>,
    pub current: Option<usize>,
    pub selected: BTreeSet<usize>,
    /// The first row shown.
    pub top: usize,
    pub focused: bool,
    pub shade: bool,
    pub pressed: Option<Control>,
    /// An open menu and the item under the pointer.
    pub menu: Option<(Menu, Option<usize>)>,
    /// "0:00/12:34"
    pub running: String,
    /// "  1:23"
    pub mini_time: String,
}

impl View {
    pub fn size(&self) -> (u32, u32) {
        if self.shade { (self.width.max(MIN_WIDTH), SHADE_HEIGHT) } else { (self.width.max(MIN_WIDTH), self.height.max(MIN_HEIGHT)) }
    }

    /// Rows that fit.
    pub fn visible_rows(&self) -> usize {
        let (_, h) = self.size();
        ((h as i32 - 20 - 38 - 6) / ROW).max(1) as usize
    }

    pub fn max_top(&self) -> usize {
        self.tracks.len().saturating_sub(self.visible_rows())
    }

    /// Scrolls so `row` is visible.
    pub fn reveal(&mut self, row: usize) {
        let rows = self.visible_rows();
        if row < self.top {
            self.top = row;
        } else if row >= self.top + rows {
            self.top = row + 1 - rows;
        }
        self.top = self.top.min(self.max_top());
    }
}

/// Window sizes snap to Winamp's 25×29 steps.
pub fn snap_size(w: i32, h: i32) -> (u32, u32) {
    let sw = ((w - MIN_WIDTH as i32).max(0) + 12) / 25;
    let sh = ((h - MIN_HEIGHT as i32).max(0) + 14) / 29;
    (MIN_WIDTH + 25 * sw as u32, MIN_HEIGHT + 29 * sh as u32)
}

pub fn hit(v: &View, x: i32, y: i32) -> Option<Control> {
    let (w, h) = v.size();
    let (w, h) = (w as i32, h as i32);
    let inside = |l: i32, t: i32, ww: i32, hh: i32| x >= l && y >= t && x < l + ww && y < t + hh;
    if inside(w - 11, 3, 9, 9) {
        return Some(Control::Close);
    }
    if inside(w - 21, 3, 9, 9) {
        return Some(Control::Shade);
    }
    if v.shade {
        if inside(w - 31, 3, 9, 9) {
            return Some(Control::Resize);
        }
        return Some(Control::TitleBar);
    }
    if y < 20 {
        return Some(Control::TitleBar);
    }
    // An open menu takes the pointer first.
    if let Some((menu, _)) = v.menu {
        let mx = menu.x(w);
        let n = menu.items().len() as i32;
        let top = h - 12 - 18 * n;
        if inside(mx, top, 22, 18 * n) {
            return Some(Control::Menu(menu));
        }
    }
    if inside(w - 20, h - 20, 20, 20) {
        return Some(Control::Resize);
    }
    if y >= h - 38 {
        for m in [Menu::Add, Menu::Remove, Menu::Select, Menu::Misc, Menu::List] {
            if inside(m.x(w), h - 30, 22, 18) {
                return Some(Control::Menu(m));
            }
        }
        let bx = w - 150 + 3;
        let minis = [Mini::Previous, Mini::Play, Mini::Pause, Mini::Stop, Mini::Next, Mini::Eject];
        for (i, m) in minis.iter().enumerate() {
            if inside(bx + i as i32 * 10, h - 38 + 22, 10, 10) {
                return Some(Control::Mini(*m));
            }
        }
        return None;
    }
    if inside(w - 15, 20, 8, h - 58) {
        return Some(Control::Scroll);
    }
    if inside(12, 20, w - 32, h - 58) {
        let row = ((y - 23) / ROW).max(0) as usize + v.top;
        return Some(if row < v.tracks.len() { Control::Track(row) } else { Control::Empty });
    }
    None
}

/// The menu item under `y`, for a menu opened in a window `h` tall.
pub fn menu_item(menu: Menu, h: i32, y: i32) -> Option<usize> {
    let n = menu.items().len() as i32;
    let top = h - 12 - 18 * n;
    (y >= top && y < top + 18 * n).then(|| ((y - top) / 18) as usize)
}

/// Where the scroll handle sits, and the first row for a pointer y.
pub fn scroll_top_at(v: &View, y: i32) -> usize {
    let (_, h) = v.size();
    let track = h as i32 - 58 - 18;
    let t = ((y - 20 - 9) as f32 / track.max(1) as f32).clamp(0.0, 1.0);
    (t * v.max_top() as f32).round() as usize
}

pub fn draw(frame: &mut Frame, skin: &Skin, v: &View) {
    let (w, h) = v.size();
    frame.resize(w, h);
    let (w, h) = (w as i32, h as i32);
    let sel = if v.focused { "_SELECTED" } else { "" };
    let st = &skin.playlist;
    if v.shade {
        frame.tile(skin, "PLAYLIST_SHADE_BACKGROUND", 25, 0, w - 75, 14);
        frame.sprite(skin, "PLAYLIST_SHADE_BACKGROUND_LEFT", 0, 0);
        frame.sprite(skin, &format!("PLAYLIST_SHADE_BACKGROUND_RIGHT{sel}"), w - 50, 0);
        if let Some(i) = v.current {
            let t = v.tracks.get(i).map(|t| format!("{}. {}", i + 1, t.title)).unwrap_or_default();
            frame.text(skin, &t.to_uppercase(), 5, 4, w - 80, 0);
            let len = v.tracks.get(i).map(|t| t.length.clone()).unwrap_or_default();
            frame.text(skin, &len, w - 30 - 5 * len.len() as i32, 4, 30, 0);
        }
        return;
    }
    // Frame: top, sides, bottom.
    frame.tile(skin, &format!("PLAYLIST_TOP_TILE{sel}"), 25, 0, w - 50, 20);
    frame.sprite(skin, &format!("PLAYLIST_TOP_LEFT{}", if v.focused { "_SELECTED" } else { "_CORNER" }), 0, 0);
    frame.sprite(skin, &format!("PLAYLIST_TITLE_BAR{sel}"), (w - 100) / 2, 0);
    frame.sprite(skin, &format!("PLAYLIST_TOP_RIGHT_CORNER{sel}"), w - 25, 0);
    frame.tile(skin, "PLAYLIST_LEFT_TILE", 0, 20, 12, h - 58);
    frame.tile(skin, "PLAYLIST_RIGHT_TILE", w - 20, 20, 20, h - 58);
    frame.tile(skin, "PLAYLIST_BOTTOM_TILE", 125, h - 38, w - 275, 38);
    frame.sprite(skin, "PLAYLIST_BOTTOM_LEFT_CORNER", 0, h - 38);
    frame.sprite(skin, "PLAYLIST_BOTTOM_RIGHT_CORNER", w - 150, h - 38);
    if w >= 350 {
        frame.sprite(skin, "PLAYLIST_VISUALIZER_BACKGROUND", w - 225, h - 38);
    }
    if v.pressed == Some(Control::Close) {
        frame.sprite(skin, "PLAYLIST_CLOSE_SELECTED", w - 11, 3);
    }
    if v.pressed == Some(Control::Shade) {
        frame.sprite(skin, "PLAYLIST_COLLAPSE_SELECTED", w - 21, 3);
    }

    // Tracks.
    let (lx, lw) = (12, w - 32);
    frame.fill(lx, 20, lw, h - 58, argb(st.normal_bg));
    let rows = v.visible_rows();
    let digits = v.tracks.len().to_string().len();
    for (n, i) in (v.top..v.tracks.len()).take(rows).enumerate() {
        let t = &v.tracks[i];
        let y = 23 + n as i32 * ROW;
        if v.selected.contains(&i) {
            frame.fill(lx, y, lw, ROW, argb(st.selected_bg));
        }
        let color = if v.current == Some(i) { st.current } else { st.normal };
        let len_w = font::measure(&t.length, FONT_PX, false);
        let num = format!("{:>width$}. ", i + 1, width = digits);
        let title = font::ellipsize(&format!("{num}{}", t.title), FONT_PX, lw - len_w - 8, false);
        font::draw(frame, lx + 2, y + 10, &title, FONT_PX, color, lw - len_w - 6, false);
        font::draw(frame, lx + lw - len_w - 3, y + 10, &t.length, FONT_PX, color, len_w + 2, false);
    }
    // Scroll handle.
    let track = h - 58 - 18;
    let hy = if v.max_top() == 0 { 0 } else { (v.top as f32 / v.max_top() as f32 * track as f32).round() as i32 };
    frame.sprite(skin, if v.pressed == Some(Control::Scroll) { "PLAYLIST_SCROLL_HANDLE_SELECTED" } else { "PLAYLIST_SCROLL_HANDLE" }, w - 15, 20 + hy);

    // Running time and mini time in the right corner.
    frame.text(skin, &v.running, w - 150 + 7, h - 38 + 10, 90, 0);
    frame.text(skin, &v.mini_time, w - 150 + 66, h - 38 + 23, 30, 0);

    // An open menu: its items above the button, with the bar beside.
    if let Some((menu, hover)) = v.menu {
        let items = menu.items();
        let mx = menu.x(w);
        let top = h - 12 - 18 * items.len() as i32;
        frame.sprite(skin, menu.bar(), mx - 3, top);
        for (i, name) in items.iter().enumerate() {
            let n = if hover == Some(i) { format!("{name}_SELECTED") } else { (*name).to_owned() };
            frame.sprite(skin, &n, mx, top + 18 * i as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        let tracks = (0..30).map(|i| Track { title: format!("Artist {i} - A song with a long name {i}"), length: "3:25".into() }).collect();
        View { width: 275, height: 116 + 29 * 3, tracks, current: Some(2), focused: true, running: "0:00/1:42:30".into(), mini_time: " 1:23".into(), ..Default::default() }
    }

    #[test]
    fn layout_and_hits() {
        let mut v = view();
        assert_eq!(snap_size(300, 150), (300, 145));
        assert_eq!(snap_size(200, 10), (275, 116));
        assert_eq!(hit(&v, 50, 25), Some(Control::Track(0)));
        assert_eq!(hit(&v, 50, 25 + ROW * 3), Some(Control::Track(3)));
        assert_eq!(hit(&v, 20, v.height as i32 - 25), Some(Control::Menu(Menu::Add)));
        assert_eq!(hit(&v, 270, 6), Some(Control::Close));
        v.top = 10;
        assert_eq!(hit(&v, 50, 25), Some(Control::Track(10)));
        v.reveal(29);
        assert_eq!(v.top, v.max_top());
    }

    #[test]
    fn draws_with_an_open_menu() {
        let skin = Skin::default_skin();
        let mut v = view();
        v.selected.insert(4);
        v.menu = Some((Menu::Remove, Some(1)));
        let mut f = Frame::new(1, 1);
        draw(&mut f, &skin, &v);
        assert_eq!((f.width, f.height), (275, 116 + 87));
        if let Ok(dir) = std::env::var("GAMP_SNAPSHOTS") {
            f.to_image().save(format!("{dir}/playlist.png")).unwrap();
        }
    }
}
