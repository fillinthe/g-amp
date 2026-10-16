//! The G-Drop window: G-Amp's MilkDrop-style visualizer in a generic
//! Winamp frame (or full screen), rendered at the screen's resolution.

use std::time::Instant;

use gamp_audio::analysis::Spectrum;
use gamp_ui::Frame;
use gamp_ui::library_window::gen_frame;
use gamp_vis::GDrop;
use winit::keyboard::{Key, NamedKey};
use winit::window::Fullscreen;

use crate::app::{App, Kind};

pub struct DropState {
    pub engine: GDrop,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub focused: bool,
    pub spectrum: Spectrum,
    pub wave: Vec<f32>,
    pub spec: Vec<f32>,
    pub last: Instant,
    /// Show the preset's name until then.
    pub name_until: Option<Instant>,
    pub pressed_close: bool,
}

impl Default for DropState {
    fn default() -> Self {
        DropState {
            engine: GDrop::new(256, 192),
            width: 275 + 25 * 4,
            height: 232,
            fullscreen: false,
            focused: false,
            spectrum: Spectrum::new(1024),
            wave: vec![0.0; 1024],
            spec: Vec::new(),
            last: Instant::now(),
            name_until: None,
            pressed_close: false,
        }
    }
}

/// Where the picture goes inside the frame.
fn inner(w: u32, h: u32, full: bool) -> (i32, i32, i32, i32) {
    if full { (0, 0, w as i32, h as i32) } else { (11, 20, w as i32 - 19, h as i32 - 34) }
}

impl App {
    pub fn drop_visible(&self) -> bool {
        self.wins.get(&Kind::Vis).is_some_and(|w| w.visible)
    }

    /// Draws a frame of the visualizer into the window.
    pub fn drop_redraw(&mut self) {
        let Some(w) = self.wins.get_mut(&Kind::Vis) else { return };
        let d = &mut self.drop;
        let (fw, fh) = if d.fullscreen { (w.frame.width.max(1), w.frame.height.max(1)) } else { (d.width, d.height) };
        if !d.fullscreen {
            w.frame.resize(fw, fh);
            gen_frame(&mut w.frame, &self.skin, fw as i32, fh as i32, "G Drop", d.focused);
            if d.pressed_close {
                w.frame.sprite(&self.skin, "GEN_CLOSE_SELECTED", fw as i32 - 11, 3);
            }
        }
        // Render at the screen's pixels (halved on big windows, which
        // the eye can't tell and the CPU can).
        let rect = inner(fw, fh, d.fullscreen);
        let (pw, _) = w.inner_px();
        let k = pw as f32 / fw.max(1) as f32;
        let (mut rw, mut rh) = ((rect.2 as f32 * k) as usize, (rect.3 as f32 * k) as usize);
        while rw * rh > 640 * 400 {
            rw /= 2;
            rh /= 2;
        }
        d.engine.resize(rw.max(32), rh.max(24));
        let now = Instant::now();
        let dt = now.duration_since(d.last).as_secs_f32();
        d.last = now;
        self.player.waveform(&mut d.wave);
        d.spectrum.compute(&d.wave, &mut d.spec);
        let rate = self.player.output_rate();
        d.engine.render(&d.wave, &d.spec, rate, dt);
        let (ew, eh) = d.engine.size();
        let mut px = d.engine.out.clone();
        // The preset's name, briefly, after a change.
        if d.name_until.is_some_and(|t| now < t) {
            let mut label = Frame { width: ew as u32, height: eh as u32, pixels: px };
            let name = d.engine.preset_name();
            gamp_ui::font::draw(&mut label, 7, eh as i32 - 7, name, 12.0, [0, 0, 0], ew as i32, true);
            gamp_ui::font::draw(&mut label, 6, eh as i32 - 8, name, 12.0, [255, 226, 120], ew as i32, true);
            px = label.pixels;
        }
        w.present_with(Some((&px, ew, eh, rect)));
    }

    fn drop_changed(&mut self) {
        self.drop.name_until = Some(Instant::now() + std::time::Duration::from_secs(3));
    }

    pub fn drop_press(&mut self, x: i32, y: i32) {
        let d = &self.drop;
        if !d.fullscreen && y < 20 {
            if x >= d.width as i32 - 11 && x < d.width as i32 - 2 && y >= 3 && y < 12 {
                self.drop.pressed_close = true;
                self.mark(Kind::Vis);
                return;
            }
            if self.double_click(Kind::Vis, x, y) {
                return self.drop_fullscreen(true);
            }
            self.start_drag(Kind::Vis);
            return;
        }
        if !d.fullscreen && x >= d.width as i32 - 20 && y >= d.height as i32 - 20 {
            if let Some(w) = self.win(Kind::Vis) {
                self.resize = Some(crate::app::Resize { start: w.global_cursor(), size: (self.drop.width, self.drop.height) });
            }
            return;
        }
        if self.double_click(Kind::Vis, x, y) {
            let full = !self.drop.fullscreen;
            return self.drop_fullscreen(full);
        }
        // A click shows the next preset.
        self.drop.engine.next(false);
        self.drop_changed();
    }

    pub fn drop_release(&mut self) {
        if std::mem::take(&mut self.drop.pressed_close) {
            let (x, y) = self.win(Kind::Vis).map(|w| w.cursor).unwrap_or_default();
            if x >= self.drop.width as i32 - 11 && y < 14 {
                self.toggle_window(Kind::Vis);
            }
            self.mark(Kind::Vis);
        }
    }

    pub fn drop_resize(&mut self) {
        let Some(r) = &self.resize else { return };
        let Some(w) = self.win(Kind::Vis) else { return };
        let now = w.global_cursor();
        let scale = w.scale();
        let nw = (r.size.0 as f64 + (now.0 - r.start.0) / scale).max(275.0) as u32;
        let nh = (r.size.1 as f64 + (now.1 - r.start.1) / scale).max(116.0) as u32;
        let (nw, nh) = (275 + (nw - 275 + 12) / 25 * 25, 116 + (nh - 116 + 14) / 29 * 29);
        if (nw, nh) != (self.drop.width, self.drop.height) {
            self.drop.width = nw;
            self.drop.height = nh;
            if let Some(w) = self.wins.get_mut(&Kind::Vis) {
                w.set_size(winit::dpi::LogicalSize::new(nw, nh));
            }
        }
    }

    pub fn drop_fullscreen(&mut self, on: bool) {
        self.drop.fullscreen = on;
        if let Some(w) = self.wins.get(&Kind::Vis).and_then(|w| w.window.clone()) {
            w.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
        }
        if !on {
            if let Some(w) = self.wins.get_mut(&Kind::Vis) {
                w.frame = Frame::new(self.drop.width, self.drop.height);
            }
        }
        self.mark(Kind::Vis);
    }

    /// Space/→ next, ← or Backspace previous, H hard cut, R random on/off,
    /// F or Enter full screen, Esc leaves full screen.
    pub fn drop_key(&mut self, key: &Key) -> bool {
        match key {
            Key::Named(NamedKey::Space | NamedKey::ArrowRight) => self.drop.engine.next(false),
            Key::Named(NamedKey::ArrowLeft | NamedKey::Backspace) => self.drop.engine.back(),
            Key::Named(NamedKey::Escape) if self.drop.fullscreen => self.drop_fullscreen(false),
            Key::Named(NamedKey::Enter) => self.drop_fullscreen(!self.drop.fullscreen),
            Key::Character(c) => match c.to_lowercase().as_str() {
                "h" => self.drop.engine.random(true),
                "r" => {
                    self.drop.engine.auto = !self.drop.engine.auto;
                    self.main.message = Some(format!("G-DROP: RANDOM {}", if self.drop.engine.auto { "ON" } else { "OFF" }));
                }
                "f" => self.drop_fullscreen(!self.drop.fullscreen),
                _ => return false,
            },
            _ => return false,
        }
        self.drop_changed();
        true
    }
}
