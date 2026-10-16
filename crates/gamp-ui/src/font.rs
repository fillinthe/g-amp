//! TrueType text for the playlist and library (Liberation Sans: Arial's
//! metrics, open licence), with a glyph cache.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use fontdue::{Font, FontSettings};

use crate::frame::Frame;

static REGULAR: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Bold.ttf");

fn fonts() -> &'static [Font; 2] {
    static F: OnceLock<[Font; 2]> = OnceLock::new();
    F.get_or_init(|| {
        [
            Font::from_bytes(REGULAR, FontSettings::default()).expect("regular font"),
            Font::from_bytes(BOLD, FontSettings::default()).expect("bold font"),
        ]
    })
}

type Glyph = (fontdue::Metrics, Vec<u8>);

fn cache() -> &'static Mutex<HashMap<(char, u32, bool), Glyph>> {
    static C: OnceLock<Mutex<HashMap<(char, u32, bool), Glyph>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn glyph(c: char, px: f32, bold: bool) -> Glyph {
    let key = (c, (px * 4.0) as u32, bold);
    let mut cache = cache().lock().unwrap();
    cache.entry(key).or_insert_with(|| fonts()[bold as usize].rasterize(c, px)).clone()
}

/// Width of `text` at `px` pixels.
pub fn measure(text: &str, px: f32, bold: bool) -> i32 {
    text.chars().map(|c| fonts()[bold as usize].metrics(c, px).advance_width).sum::<f32>().round() as i32
}

#[inline]
fn blend(dst: u32, color: [u8; 3], a: u8) -> u32 {
    let a = a as u32;
    let mix = |d: u32, c: u8| (d * (255 - a) + c as u32 * a) / 255;
    let (r, g, b) = ((dst >> 16) & 0xFF, (dst >> 8) & 0xFF, dst & 0xFF);
    0xFF00_0000 | mix(r, color[0]) << 16 | mix(g, color[1]) << 8 | mix(b, color[2])
}

/// Draws `text` with its baseline at `y`, clipped to `[x, x + max_w)`.
/// Returns the width drawn.
pub fn draw(frame: &mut Frame, x: i32, y: i32, text: &str, px: f32, color: [u8; 3], max_w: i32, bold: bool) -> i32 {
    let mut pen = x as f32;
    let limit = x + max_w;
    for c in text.chars() {
        let (m, bitmap) = glyph(c, px, bold);
        let gx = pen.round() as i32 + m.xmin;
        let gy = y - m.height as i32 - m.ymin;
        if gx + m.width as i32 > limit {
            break;
        }
        for row in 0..m.height {
            for col in 0..m.width {
                let a = bitmap[row * m.width + col];
                if a == 0 {
                    continue;
                }
                let (px_, py) = (gx + col as i32, gy + row as i32);
                if px_ < x || px_ >= limit || px_ < 0 || py < 0 || px_ as u32 >= frame.width || py as u32 >= frame.height {
                    continue;
                }
                let i = (py as u32 * frame.width + px_ as u32) as usize;
                frame.pixels[i] = blend(frame.pixels[i], color, a);
            }
        }
        pen += m.advance_width;
    }
    (pen - x as f32).round() as i32
}

/// `text` shortened with "…" to fit `max_w`.
pub fn ellipsize(text: &str, px: f32, max_w: i32, bold: bool) -> String {
    if measure(text, px, bold) <= max_w {
        return text.to_owned();
    }
    let mut s: String = text.to_owned();
    while !s.is_empty() && measure(&format!("{s}…"), px, bold) > max_w {
        s.pop();
    }
    format!("{s}…")
}
