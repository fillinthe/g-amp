//! One OS window and its pixels. In tests there is no OS window: the same
//! code runs and only the presenting is skipped.

use std::num::NonZeroU32;
use std::rc::Rc;

use gamp_ui::Frame;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

type Surface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

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
        for y in 0..bh {
            let sy = ((y as u64 * fh as u64 / bh as u64) as u32).min(fh - 1);
            let row = (sy * fw) as usize;
            let src = &self.frame.pixels[row..row + fw as usize];
            let dst = &mut buf[(y * bw) as usize..((y + 1) * bw) as usize];
            for (x, d) in dst.iter_mut().enumerate() {
                *d = src[((x as u64 * fw as u64 / bw as u64) as usize).min(fw as usize - 1)] & 0x00FF_FFFF;
            }
        }
        let _ = buf.present();
        self.dirty = false;
    }
}
