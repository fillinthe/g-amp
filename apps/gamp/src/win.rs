//! One OS window and its pixels. In tests there is no OS window: the same
//! code runs and only the presenting is skipped.

use std::num::NonZeroU32;
use std::rc::Rc;

use gamp_ui::Frame;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

type Surface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

/// G-Amp's icon for the taskbar (Windows, Linux).
fn icon() -> Option<winit::window::Icon> {
    static PNG: &[u8] = include_bytes!("../../../assets/logo/gamp-icon-256.png");
    let img = image::load_from_memory(PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

pub struct Win {
    pub window: Option<Rc<Window>>,
    surface: Option<Surface>,
    pub frame: Frame,
    /// Pointer in frame (1×) pixels.
    pub cursor: (i32, i32),
    /// Pointer in the window's physical pixels.
    pub local: (f64, f64),
    /// Outer position, physical pixels (tracked; set when we move it).
    pub pos: (i32, i32),
    pub visible: bool,
    pub dirty: bool,
}

impl Win {
    pub fn headless() -> Win {
        Win { window: None, surface: None, frame: Frame::new(1, 1), cursor: (0, 0), local: (0.0, 0.0), pos: (0, 0), visible: true, dirty: true }
    }

    /// A borderless window `size` logical pixels big at `pos` (physical).
    pub fn open(el: Option<&ActiveEventLoop>, title: &str, size: LogicalSize<u32>, pos: (i32, i32), popup: bool) -> Win {
        let mut w = Win::headless();
        w.pos = pos;
        let Some(el) = el else { return w };
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(size)
            .with_position(PhysicalPosition::new(pos.0, pos.1));
        if popup {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        } else {
            attrs = attrs.with_window_icon(icon());
        }
        match el.create_window(attrs) {
            Ok(window) => {
                let window = Rc::new(window);
                let surface = softbuffer::Context::new(window.clone()).ok().and_then(|ctx| Surface::new(&ctx, window.clone()).ok());
                if let Ok(p) = window.outer_position() {
                    w.pos = (p.x, p.y);
                }
                w.window = Some(window);
                w.surface = surface;
            }
            Err(e) => log::warn!("can't open the {title} window: {e}"),
        }
        w
    }

    /// Inner size in physical pixels.
    pub fn inner_px(&self) -> (u32, u32) {
        match &self.window {
            Some(w) => {
                let s = w.inner_size();
                (s.width, s.height)
            }
            None => (self.frame.width, self.frame.height),
        }
    }

    pub fn scale(&self) -> f64 {
        self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0)
    }

    /// Outer size in physical pixels.
    pub fn size_px(&self) -> (i32, i32) {
        match &self.window {
            Some(w) => {
                let s = w.outer_size();
                (s.width as i32, s.height as i32)
            }
            None => (self.frame.width as i32, self.frame.height as i32),
        }
    }

    pub fn set_size(&mut self, size: LogicalSize<u32>) {
        if let Some(w) = &self.window {
            let _ = w.request_inner_size(size);
        }
        self.dirty = true;
    }

    pub fn move_to(&mut self, x: i32, y: i32) {
        self.pos = (x, y);
        if let Some(w) = &self.window {
            w.set_outer_position(PhysicalPosition::new(x, y));
        }
    }

    pub fn show(&mut self, on: bool) {
        self.visible = on;
        if let Some(w) = &self.window {
            w.set_visible(on);
        }
        self.dirty = true;
    }

    pub fn request_redraw(&self) {
        if let Some(w) = &self.window {
            if self.visible {
                w.request_redraw();
            }
        }
    }

    pub fn set_cursor(&mut self, x: f64, y: f64) {
        self.local = (x, y);
        let (ww, wh) = match &self.window {
            Some(w) => {
                let s = w.inner_size();
                (s.width.max(1) as f64, s.height.max(1) as f64)
            }
            None => (self.frame.width as f64, self.frame.height as f64),
        };
        self.cursor = ((x * self.frame.width as f64 / ww) as i32, (y * self.frame.height as f64 / wh) as i32);
    }

    /// Pointer position on the screen, physical pixels.
    pub fn global_cursor(&self) -> (f64, f64) {
        (self.pos.0 as f64 + self.local.0, self.pos.1 as f64 + self.local.1)
    }

    /// Copies the frame to the window, scaled up sharply to fill it.
    pub fn present(&mut self) {
        self.present_with(None);
    }

    /// Like `present`, with full-resolution pixels (a visualizer) laid
    /// over `rect` (frame coordinates), smoothly scaled.
    pub fn present_with(&mut self, overlay: Option<(&[u32], usize, usize, (i32, i32, i32, i32))>) {
        let (Some(window), Some(surface)) = (&self.window, self.surface.as_mut()) else {
            self.dirty = false;
            return;
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let (fw, fh) = (self.frame.width.max(1), self.frame.height.max(1));
        let (bw, bh) = (size.width, size.height);
        // Where an overlay goes, in window pixels; the frame isn't drawn there.
        let hole = overlay.as_ref().map(|(_, _, _, (rx, ry, rw, rh))| {
            let (sx, sy) = (bw as f32 / fw as f32, bh as f32 / fh as f32);
            ((*rx as f32 * sx) as i32, (*ry as f32 * sy) as i32, (*rw as f32 * sx) as i32, (*rh as f32 * sy) as i32)
        });
        let cols: Vec<usize> = (0..bw).map(|x| ((x as u64 * fw as u64 / bw as u64) as usize).min(fw as usize - 1)).collect();
        for y in 0..bh {
            let sy = ((y as u64 * fh as u64 / bh as u64) as u32).min(fh - 1);
            let row = (sy * fw) as usize;
            let src = &self.frame.pixels[row..row + fw as usize];
            let dst = &mut buf[(y * bw) as usize..((y + 1) * bw) as usize];
            match hole {
                Some((hx, hy, hw, hh)) if (y as i32) >= hy && (y as i32) < hy + hh => {
                    for x in (0..hx.max(0) as usize).chain(((hx + hw).max(0) as usize).min(bw as usize)..bw as usize) {
                        dst[x] = src[cols[x]] & 0x00FF_FFFF;
                    }
                }
                _ => {
                    for (d, c) in dst.iter_mut().zip(&cols) {
                        *d = src[*c] & 0x00FF_FFFF;
                    }
                }
            }
        }
        if let (Some((px, ow, oh, _)), Some((x0, y0, pw, ph))) = (overlay, hole) {
            // Bilinear in fixed point, with the column maths done once.
            let xs: Vec<(usize, usize, u32)> = (0..pw.max(0))
                .map(|x| {
                    let u = ((x as f32 + 0.5) * ow as f32 / pw as f32 - 0.5).max(0.0);
                    let ux = (u as usize).min(ow - 1);
                    (ux, (ux + 1).min(ow - 1), (u.fract() * 256.0) as u32)
                })
                .collect();
            for y in 0..ph.max(0) {
                let ty = y0 + y;
                if ty < 0 || ty as u32 >= bh {
                    continue;
                }
                let v = ((y as f32 + 0.5) * oh as f32 / ph as f32 - 0.5).max(0.0);
                let vy = (v as usize).min(oh - 1);
                let vy1 = (vy + 1).min(oh - 1);
                let fy = (v.fract() * 256.0) as u32;
                let (r0, r1) = (&px[vy * ow..vy * ow + ow], &px[vy1 * ow..vy1 * ow + ow]);
                let line = &mut buf[(ty as u32 * bw) as usize..((ty as u32 + 1) * bw) as usize];
                for (x, &(ux, ux1, fx)) in xs.iter().enumerate() {
                    let tx = x0 + x as i32;
                    if tx < 0 || tx as u32 >= bw {
                        continue;
                    }
                    let (a, b, c, d) = (r0[ux], r0[ux1], r1[ux], r1[ux1]);
                    // Two channels at a time (red+blue, then green).
                    let mix = |p: u32, q: u32, f: u32, m: u32| (((p & m) * (256 - f) + (q & m) * f) >> 8) & m;
                    let rb_top = mix(a, b, fx, 0x00FF_00FF);
                    let rb_bot = mix(c, d, fx, 0x00FF_00FF);
                    let g_top = mix(a, b, fx, 0x0000_FF00);
                    let g_bot = mix(c, d, fx, 0x0000_FF00);
                    line[tx as usize] = mix(rb_top, rb_bot, fy, 0x00FF_00FF) | mix(g_top, g_bot, fy, 0x0000_FF00);
                }
            }
        }
        let _ = buf.present();
        self.dirty = false;
    }
}
