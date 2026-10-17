//! File Info (Alt+3): a generic Winamp window listing a track's tags and
//! format, label and value per row, with its path at the bottom.

use gamp_skin::Skin;

use crate::font;
use crate::frame::{Frame, argb};
use crate::library_window::{self as lw, Hot, PX, R, ROW, button, button_w, gen_frame, mix};

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 232;
const LABEL_W: i32 = 80;

#[derive(Clone, Debug, Default)]
pub struct View {
    pub focused: bool,
    /// (label, value) rows; an empty label is a heading.
    pub rows: Vec<(String, String)>,
    /// The file's path or address, wrapped at the bottom.
    pub place: String,
    pub pressed: Option<Hot>,
}

fn inner() -> R {
    (11, 20, WIDTH as i32 - 19, HEIGHT as i32 - 34)
}

pub fn draw(frame: &mut Frame, skin: &Skin, v: &View) -> Vec<(Hot, R)> {
    let (w, h) = (WIDTH, HEIGHT);
    frame.resize(w, h);
    let (w, h) = (w as i32, h as i32);
    let st = &skin.playlist;
    let mut hots: Vec<(Hot, R)> = Vec::new();
    gen_frame(frame, skin, w, h, "File Info", v.focused);
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

    // The panel: rows, then the place.
    let p = (ix + 2, iy + 2, iw - 4, ih - 4 - 22);
    frame.fill(p.0, p.1, p.2, p.3, argb(st.normal_bg));
    let dim = mix(st.normal, st.normal_bg, 0.45);
    let mut y = p.1 + 2;
    let bottom = p.1 + p.3;
    for (label, value) in &v.rows {
        if y + ROW > bottom {
            break;
        }
        if label.is_empty() {
            font::draw(frame, p.0 + 4, y + 11, value, PX, st.current, p.2 - 8, true);
        } else {
            font::draw(frame, p.0 + 4, y + 11, label, PX, dim, LABEL_W - 4, false);
            let text = font::ellipsize(value, PX, p.2 - LABEL_W - 8, false);
            font::draw(frame, p.0 + LABEL_W, y + 11, &text, PX, st.normal, p.2 - LABEL_W - 8, false);
        }
        y += ROW;
    }
    // The place, wrapped, in the room left.
    let lines = lw::wrap(&v.place, p.2 - 8);
    let room = ((bottom - y - 2) / (ROW - 2)).max(0) as usize;
    for (n, line) in lines.iter().take(room).enumerate() {
        let shown = if n + 1 == room && lines.len() > room { font::ellipsize(&format!("{line}…"), PX, p.2 - 8, false) } else { line.clone() };
        font::draw(frame, p.0 + 4, y + 11 + n as i32 * (ROW - 2), &shown, PX, dim, p.2 - 8, false);
    }

    let by = p.1 + p.3 + 4;
    let bw = button_w("Close");
    let r = (ix + iw - 2 - bw, by, bw, 16);
    button(frame, r, "Close", v.pressed == Some(Hot::Button("close".into())), true);
    hots.push((Hot::Button("close".into()), r));
    hots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_rows_and_the_place() {
        let skin = Skin::default_skin();
        let v = View {
            rows: vec![(String::new(), "Around the World".into()), ("Artist".into(), "Daft Punk".into()), ("Length".into(), "7:09".into())],
            place: "/Volumes/Music/Daft Punk/Homework/07 Around the World.mp3".into(),
            ..Default::default()
        };
        let mut f = Frame::new(1, 1);
        let hots = draw(&mut f, &skin, &v);
        assert!(hots.iter().any(|(h, _)| *h == Hot::Close));
        assert!(hots.iter().any(|(h, _)| *h == Hot::Button("close".into())));
        assert_eq!((f.width, f.height), (WIDTH, HEIGHT));
    }
}

