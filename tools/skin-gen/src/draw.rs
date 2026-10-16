//! A small pixel canvas.

use image::{Rgba, RgbaImage};

pub type Rgb = [u8; 3];

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

pub struct Canvas {
    pub img: RgbaImage,
}

impl Canvas {
    pub fn new(w: u32, h: u32, fill: Rgb) -> Canvas {
        Canvas { img: RgbaImage::from_pixel(w, h, Rgba([fill[0], fill[1], fill[2], 255])) }
    }

    pub fn px(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && (x as u32) < self.img.width() && (y as u32) < self.img.height() {
            self.img.put_pixel(x as u32, y as u32, Rgba([c[0], c[1], c[2], 255]));
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.px(xx, yy, c);
            }
        }
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, c: Rgb) {
        self.rect(x, y, w, 1, c);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, c: Rgb) {
        self.rect(x, y, 1, h, c);
    }

    /// A raised edge: light top/left, dark bottom/right.
    pub fn raised(&mut self, x: i32, y: i32, w: i32, h: i32, hi: Rgb, lo: Rgb) {
        self.hline(x, y, w, hi);
        self.vline(x, y, h, hi);
        self.hline(x, y + h - 1, w, lo);
        self.vline(x + w - 1, y, h, lo);
    }

    /// A sunken edge.
    pub fn sunken(&mut self, x: i32, y: i32, w: i32, h: i32, hi: Rgb, lo: Rgb) {
        self.raised(x, y, w, h, lo, hi);
    }

    pub fn vgrad(&mut self, x: i32, y: i32, w: i32, h: i32, top: Rgb, bottom: Rgb) {
        for i in 0..h {
            let t = if h > 1 { i as f32 / (h - 1) as f32 } else { 0.0 };
            self.hline(x, y + i, w, mix(top, bottom, t));
        }
    }

    /// Pattern rows: '#' ink, '+' second colour, anything else untouched.
    pub fn pattern(&mut self, x: i32, y: i32, rows: &[&str], ink: Rgb, second: Rgb) {
        for (dy, row) in rows.iter().enumerate() {
            for (dx, ch) in row.chars().enumerate() {
                match ch {
                    '#' => self.px(x + dx as i32, y + dy as i32, ink),
                    '+' => self.px(x + dx as i32, y + dy as i32, second),
                    _ => {}
                }
            }
        }
    }

    /// Text in the skin's pixel font; returns the width drawn.
    pub fn text(&mut self, x: i32, y: i32, s: &str, c: Rgb) -> i32 {
        let mut cx = x;
        for ch in s.chars() {
            if ch == ' ' {
                cx += 3;
                continue;
            }
            self.pattern(cx, y, crate::font::glyph(ch), c, c);
            cx += crate::font::width(ch) as i32 + 1;
        }
        cx - x - 1
    }

}

pub fn text_width(s: &str) -> i32 {
    s.chars().map(|c| if c == ' ' { 3 } else { crate::font::width(c) as i32 + 1 }).sum::<i32>() - 1
}
