//! A window's pixels at 1× size (0xAARRGGBB), and drawing from skins.

use gamp_skin::Skin;

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

#[inline]
pub fn argb(c: [u8; 3]) -> u32 {
    0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

impl Frame {
    pub fn new(width: u32, height: u32) -> Frame {
        Frame { width, height, pixels: vec![0xFF00_0000; (width * height) as usize] }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != (self.width, self.height) {
            *self = Frame::new(width, height);
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            self.pixels[(y as u32 * self.width + x as u32) as usize] = c;
        }
    }

    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        for yy in y.max(0)..(y + h).min(self.height as i32) {
            for xx in x.max(0)..(x + w).min(self.width as i32) {
                self.pixels[(yy as u32 * self.width + xx as u32) as usize] = c;
            }
        }
    }

    /// Part of a sprite: `(sx, sy)` inside it, `w`×`h`, drawn at `(dx, dy)`.
    pub fn sprite_part(&mut self, skin: &Skin, name: &str, sx: u32, sy: u32, w: u32, h: u32, dx: i32, dy: i32) -> bool {
        let Some((img, r)) = skin.sprite(name) else { return false };
        let w = w.min(r.width.saturating_sub(sx));
        let h = h.min(r.height.saturating_sub(sy));
        for y in 0..h {
            let ty = dy + y as i32;
            if ty < 0 || ty as u32 >= self.height {
                continue;
            }
            for x in 0..w {
                let tx = dx + x as i32;
                if tx < 0 || tx as u32 >= self.width {
                    continue;
                }
                let p = img.get_pixel(r.x + sx + x, r.y + sy + y).0;
                self.pixels[(ty as u32 * self.width + tx as u32) as usize] = argb([p[0], p[1], p[2]]);
            }
        }
        true
    }

    pub fn sprite(&mut self, skin: &Skin, name: &str, dx: i32, dy: i32) -> bool {
        self.sprite_part(skin, name, 0, 0, u32::MAX, u32::MAX, dx, dy)
    }

    /// A sprite repeated across `w`×`h`.
    pub fn tile(&mut self, skin: &Skin, name: &str, dx: i32, dy: i32, w: i32, h: i32) {
        let Some((_, r)) = skin.sprite(name) else { return };
        let mut y = 0;
        while y < h {
            let mut x = 0;
            while x < w {
                self.sprite_part(skin, name, 0, 0, (w - x) as u32, (h - y) as u32, dx + x, dy + y);
                x += r.width as i32;
            }
            y += r.height as i32;
        }
    }

    /// Text in the skin's TEXT.BMP font (5×6 cells), clipped to `max_w`.
    /// `scroll` shifts it left by that many pixels, wrapping around.
    pub fn text(&mut self, skin: &Skin, text: &str, dx: i32, dy: i32, max_w: i32, scroll: i32) {
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let total = chars.len() as i32 * 5;
        let start = if total > 0 { scroll.rem_euclid(total) } else { 0 };
        let mut x = 0;
        while x < max_w {
            let p = (start + x) % total.max(1);
            let ci = (p / 5) as usize;
            let cx = p % 5;
            let ch = chars[ci];
            let w = (5 - cx).min(max_w - x);
            // A letter the skin lacks shows without its accent (É as E).
            let drawn = [ch, plain(ch)].iter().any(|c| self.sprite_part(skin, &format!("CHARACTER_{}", *c as u32), cx as u32, 0, w as u32, 6, dx + x, dy));
            if !drawn {
                self.sprite_part(skin, "CHARACTER_32", cx as u32, 0, w as u32, 6, dx + x, dy);
            }
            x += w;
            if total <= max_w && x >= total {
                break;
            }
        }
    }

    /// For tests and screenshots.
    pub fn to_image(&self) -> image::RgbaImage {
        image::RgbaImage::from_fn(self.width, self.height, |x, y| {
            let p = self.pixels[(y * self.width + x) as usize];
            image::Rgba([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
        })
    }
}

/// `c` without its accent, for the skin's A-to-Z font.
pub(crate) fn plain(c: char) -> char {
    const FROM: &str = "ÀÁÂÃÄÅàáâãäåÇçÈÉÊËèéêëÌÍÎÏìíîïÑñÒÓÔÕÖòóôõöÙÚÛÜùúûüÝýÿß";
    const TO: &str = "AAAAAAaaaaaaCcEEEEeeeeIIIIiiiiNnOOOOOoooooUUUUuuuuYyys";
    FROM.chars().position(|f| f == c).and_then(|i| TO.chars().nth(i)).unwrap_or(c)
}
