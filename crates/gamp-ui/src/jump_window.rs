//! Jump to File (J): a generic Winamp window with a search box and the
//! tracks in the playlist that match what you typed. Enter plays one.

use gamp_skin::Skin;

use crate::font;
use crate::frame::{Frame, argb};
use crate::library_window::{self as lw, Field, Hot, PX, R, ROW, button, button_w, field, gen_frame, mix};

pub const WIDTH: u32 = 300;
pub const HEIGHT: u32 = 232;

#[derive(Clone, Debug, Default)]
pub struct Row {
    /// "12. Artist - Title"
    pub text: String,
    /// The one playing now.
    pub current: bool,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub focused: bool,
    pub search: Field,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub top: usize,
    /// "3 of 120"
    pub status: String,
    pub pressed: Option<Hot>,
}

fn inner() -> R {
    (11, 20, WIDTH as i32 - 19, HEIGHT as i32 - 34)
}

/// Where the list is.
pub fn list_rect() -> R {
    let (x, y, w, h) = inner();
    (x + 2, y + 22, w - 4, h - 22 - 24)
}

impl View {
    pub fn visible_rows(&self) -> usize {
        ((list_rect().3 - 4) / ROW).max(1) as usize
    }

    pub fn max_top(&self) -> usize {
        self.rows.len().saturating_sub(self.visible_rows())
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

pub fn draw(frame: &mut Frame, skin: &Skin, v: &View) -> Vec<(Hot, R)> {
    let (w, h) = (WIDTH, HEIGHT);
    frame.resize(w, h);
    let (w, h) = (w as i32, h as i32);
    let st = &skin.playlist;
    let mut hots: Vec<(Hot, R)> = Vec::new();
    gen_frame(frame, skin, w, h, "Jump to File", v.focused);
    if v.pressed == Some(Hot::Close) {
        frame.sprite(skin, "GEN_CLOSE_SELECTED", w - 11, 3);
    }
    hots.push((Hot::Close, lw::close_rect(w)));

    let face = skin.sprite("GEN_MIDDLE_LEFT").map(|(img, r)| {
        let p = img.get_pixel(r.x + r.width - 1, r.y + r.height / 2).0;
        argb([p[0], p[1], p[2]])
    });
    let (ix, iy, iw, ih) = inner();
    frame.fill(ix, iy, iw, ih, face.unwrap_or(0xFF2A_2940));

    // The search box.
    let sr = (ix + 2, iy + 2, iw - 4, 16);
    field(frame, skin, sr, &v.search);
    hots.push((Hot::Search, sr));

    // The matches.
    let l = list_rect();
    frame.fill(l.0, l.1, l.2, l.3, argb(st.normal_bg));
    let usable = l.2 - 10;
    for (n, i) in (v.top..v.rows.len()).take(v.visible_rows()).enumerate() {
        let row = &v.rows[i];
        let y = l.1 + 2 + n as i32 * ROW;
        let r = (l.0, y, usable, ROW);
        let sel = v.selected == Some(i);
        if sel {
            frame.fill(r.0, r.1, r.2, r.3, argb(st.selected_bg));
        }
        let col = if sel || row.current { st.current } else { st.normal };
        let text = font::ellipsize(&row.text, PX, usable - 8, false);
        font::draw(frame, r.0 + 4, y + 11, &text, PX, col, usable - 8, false);
        hots.push((Hot::Row(i), r));
    }
    if v.rows.len() > v.visible_rows() {
        // A thin scroll indicator.
        let total = v.rows.len() as i32;
        let th = (l.3 * v.visible_rows() as i32 / total).max(12);
        let ty = l.1 + ((l.3 - th) as f32 * v.top as f32 / v.max_top().max(1) as f32) as i32;
        frame.fill(l.0 + l.2 - 6, ty, 3, th, argb(mix(st.normal, st.normal_bg, 0.5)));
    }

    // Status and buttons.
    let by = l.1 + l.3 + 4;
    font::draw(frame, ix + 4, by + 12, &v.status, PX, st.normal, iw / 2, false);
    let mut x = ix + iw - 2;
    for (id, label) in [("close", "Close"), ("play", "Play")] {
        let bw = button_w(label);
        x -= bw;
        let r = (x, by, bw, 16);
        button(frame, r, label, v.pressed == Some(Hot::Button(id.into())), true);
        hots.push((Hot::Button(id.into()), r));
        x -= 6;
    }
    hots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_rows_and_buttons() {
        let skin = Skin::default_skin();
        let mut v = View::default();
        v.rows = (0..40).map(|i| Row { text: format!("{}. Song {i}", i + 1), current: i == 3 }).collect();
        v.selected = Some(30);
        v.reveal(30);
        v.status = "40 of 40".into();
        let mut f = Frame::new(1, 1);
        let hots = draw(&mut f, &skin, &v);
        assert!(hots.iter().any(|(h, _)| *h == Hot::Row(30)));
        assert!(!hots.iter().any(|(h, _)| *h == Hot::Row(0)), "scrolled past the first rows");
        assert!(hots.iter().any(|(h, _)| *h == Hot::Button("play".into())));
        assert!(v.top > 0 && v.top <= v.max_top());
    }
}
