//! File Info (Alt+3): a generic Winamp window with a track's tags in
//! editable fields (for files), its format and length, and its path.

use gamp_skin::Skin;

use crate::font;
use crate::frame::{Frame, argb};
use crate::library_window::{self as lw, Field, Hot, PX, R, ROW, button, button_w, field, gen_frame, mix};

pub const WIDTH: u32 = 320;
/// The least height; it grows to fit long values, up to MAX_HEIGHT.
pub const HEIGHT: u32 = 300;
const MAX_HEIGHT: u32 = 640;
const LABEL_W: i32 = 80;
const FIELD_H: i32 = 17;
const LINE: i32 = ROW - 2;

#[derive(Clone, Debug, Default)]
pub struct View {
    pub focused: bool,
    /// The heading: "7. Around the World".
    pub heading: String,
    /// Editable tags: (label, field). Empty for tracks that can't be written.
    pub fields: Vec<(String, Field)>,
    /// Read-only (label, value) rows under the fields.
    pub rows: Vec<(String, String)>,
    /// The file's path or address, wrapped at the bottom.
    pub place: String,
    /// "Saved", or why not.
    pub status: String,
    pub pressed: Option<Hot>,
}

impl View {
    /// Everything shown, as text to copy.
    pub fn text(&self) -> String {
        let mut out = vec![self.heading.clone()];
        out.extend(self.fields.iter().filter(|(_, f)| !f.value.is_empty()).map(|(l, f)| format!("{l}: {}", f.value)));
        out.extend(self.rows.iter().map(|(l, v)| format!("{l}: {v}")));
        if !self.place.is_empty() {
            out.push(self.place.clone());
        }
        out.join("\n")
    }
}

const PANEL_W: i32 = WIDTH as i32 - 19 - 4;
fn value_w() -> i32 {
    PANEL_W - LABEL_W - 8
}

/// How tall the window is for what it shows: everything wrapped, nothing cut.
pub fn height(v: &View) -> u32 {
    let rows: i32 = v.rows.iter().map(|(_, val)| lw::wrap(val, value_w()).len().max(1) as i32 * LINE + (ROW - LINE)).sum();
    let place = lw::wrap(&v.place, PANEL_W - 8).len() as i32 * LINE;
    let heading = lw::wrap(&v.heading, PANEL_W - 8).len().min(2) as i32 * LINE + 4;
    let fields = v.fields.len() as i32 * FIELD_H + if v.fields.is_empty() { 0 } else { 2 };
    // Frame, panel padding, then the status and buttons line.
    let need = 20 + 2 + 2 + heading + fields + rows + 2 + place + 4 + 22 + 14 + 4;
    (need.max(0) as u32).clamp(HEIGHT, MAX_HEIGHT)
}

pub fn draw(frame: &mut Frame, skin: &Skin, v: &View) -> Vec<(Hot, R)> {
    let (w, h) = (WIDTH, height(v));
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
    let (ix, iy, iw, ih) = (11, 20, w - 19, h - 34);
    frame.fill(ix, iy, iw, ih, face.unwrap_or(0xFF2A_2940));

    // The panel: heading, fields, rows, then the place, all wrapped.
    let p = (ix + 2, iy + 2, iw - 4, ih - 4 - 22);
    frame.fill(p.0, p.1, p.2, p.3, argb(st.normal_bg));
    let dim = mix(st.normal, st.normal_bg, 0.45);
    let bottom = p.1 + p.3;
    let mut y = p.1 + 2;
    let heading = lw::wrap(&v.heading, p.2 - 8);
    for (n, line) in heading.iter().take(2).enumerate() {
        let shown = if n == 1 && heading.len() > 2 { font::ellipsize(&format!("{line}…"), PX, p.2 - 8, true) } else { line.clone() };
        font::draw(frame, p.0 + 4, y + 11, &shown, PX, st.current, p.2 - 8, true);
        y += LINE;
    }
    y += 4;
    for (label, f) in &v.fields {
        if y + FIELD_H > bottom {
            break;
        }
        font::draw(frame, p.0 + 4, y + 12, crate::lang::t(label), PX, dim, LABEL_W - 4, false);
        let r = (p.0 + LABEL_W, y, p.2 - LABEL_W - 4, FIELD_H - 1);
        field(frame, skin, r, f);
        hots.push((Hot::Form(label.clone()), r));
        y += FIELD_H;
    }
    if !v.fields.is_empty() {
        y += 2;
    }
    for (label, value) in &v.rows {
        if y + LINE > bottom {
            break;
        }
        font::draw(frame, p.0 + 4, y + 11, crate::lang::t(label), PX, dim, LABEL_W - 4, false);
        for line in lw::wrap(value, p.2 - LABEL_W - 8) {
            if y + LINE > bottom {
                break;
            }
            font::draw(frame, p.0 + LABEL_W, y + 11, &line, PX, st.normal, p.2 - LABEL_W - 8, false);
            y += LINE;
        }
        y += ROW - LINE;
    }
    // The place, wrapped, in the room left (all of it, unless it's huge).
    let lines = lw::wrap(&v.place, p.2 - 8);
    let room = ((bottom - y - 2) / LINE).max(0) as usize;
    for (n, line) in lines.iter().take(room).enumerate() {
        let shown = if n + 1 == room && lines.len() > room { font::ellipsize(&format!("{line}…"), PX, p.2 - 8, false) } else { line.clone() };
        font::draw(frame, p.0 + 4, y + 11 + n as i32 * LINE, &shown, PX, dim, p.2 - 8, false);
    }

    // Status and buttons.
    let by = p.1 + p.3 + 4;
    font::draw(frame, ix + 4, by + 12, crate::lang::t(&v.status), PX, st.normal, iw / 3, false);
    let mut x = ix + iw - 2;
    let buttons: &[(&str, &str)] = if v.fields.is_empty() { &[("close", "Close"), ("copy", "Copy")] } else { &[("close", "Cancel"), ("save", "Save"), ("copy", "Copy")] };
    for (id, label) in buttons {
        let bw = button_w(label);
        x -= bw;
        let r = (x, by, bw, 16);
        button(frame, r, label, v.pressed == Some(Hot::Button((*id).into())), true);
        hots.push((Hot::Button((*id).into()), r));
        x -= 6;
    }
    hots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_fields_rows_and_buttons() {
        let skin = Skin::default_skin();
        let mut v = View {
            heading: "7. Around the World".into(),
            rows: vec![("Format".into(), "MP3, 320 kbps".into()), ("Length".into(), "7:09".into())],
            place: "/Volumes/Music/Daft Punk/Homework/07 Around the World.mp3".into(),
            ..Default::default()
        };
        let mut f = Frame::new(1, 1);
        let hots = draw(&mut f, &skin, &v);
        assert!(hots.iter().any(|(h, _)| *h == Hot::Button("close".into())));
        assert!(!hots.iter().any(|(h, _)| *h == Hot::Button("save".into())), "nothing to save");
        v.fields = vec![("Title".into(), Field { value: "Around the World".into(), focused: true, caret: 3, ..Default::default() }), ("Artist".into(), Field::new(""))];
        let hots = draw(&mut f, &skin, &v);
        assert!(hots.iter().any(|(h, _)| *h == Hot::Form("Artist".into())));
        assert!(hots.iter().any(|(h, _)| *h == Hot::Button("save".into())));
        assert_eq!((f.width, f.height), (WIDTH, HEIGHT));
        assert!(hots.iter().any(|(h, _)| *h == Hot::Button("copy".into())));
    }

    #[test]
    fn long_values_wrap_and_the_window_grows_to_fit() {
        let skin = Skin::default_skin();
        let url = "http://127.0.0.1:8742/media/soundcloud/web%3A2353638236.mp3?sig=abcdef0123456789abcdef0123456789&expires=1761320000&from=5400.000";
        let mut v = View { heading: "1. Bluebird | Migueliste".into(), rows: vec![("Title".into(), "Bluebird | Migueliste, a very long title that does not fit on one line at all".into()), ("Source".into(), "Network track".into())], place: url.into(), ..Default::default() };
        // The address breaks into lines that each fit, and nothing is lost.
        let lines = lw::wrap(url, PANEL_W - 8);
        assert!(lines.len() > 2);
        assert!(lines.iter().all(|l| font::measure(l, PX, false) <= PANEL_W - 8), "{lines:?}");
        assert_eq!(lines.concat(), url);
        let short = height(&v);
        v.rows.push(("Comment".into(), "word ".repeat(80)));
        let tall = height(&v);
        assert!(tall > short && tall <= MAX_HEIGHT);
        let mut f = Frame::new(1, 1);
        draw(&mut f, &skin, &v);
        assert_eq!(f.height, tall);
        if let Ok(dir) = std::env::var("GAMP_SNAPSHOTS") {
            f.to_image().save(format!("{dir}/info.png")).unwrap();
        }
        // Copy: every value whole, on its own line.
        let text = v.text();
        assert!(text.contains(url) && text.contains("Title: Bluebird | Migueliste, a very long title"));
    }
}
