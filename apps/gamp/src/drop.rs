//! The G-Drop window: Gamp's MilkDrop-style visualizer in a generic
//! Winamp frame (or full screen), rendered at the screen's resolution.

use std::time::Instant;

use gamp_audio::analysis::Spectrum;
use gamp_ui::Frame;
use gamp_ui::menu::Item;
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
    /// Average time a frame takes to make and show, in ms.
    pub cost_ms: f32,
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
            cost_ms: 0.0,
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

    /// Animates only while music plays and it can be seen; stopped or
    /// paused it holds its last frame, as Winamp's visualizers did.
    pub fn drop_animating(&self) -> bool {
        self.main.state == gamp_ui::main_window::PlayState::Playing && self.wins.get(&Kind::Vis).is_some_and(|w| w.visible && !w.occluded)
    }

    /// Draws a frame of the visualizer into the window.
    /// 60 frames a second when the machine keeps up easily, else 30.
    pub fn drop_tick(&self) -> std::time::Duration {
        if self.drop.cost_ms > 7.0 { crate::app::TICK } else { crate::app::VIS_TICK }
    }

    pub fn drop_redraw(&mut self) {
        let started = Instant::now();
        self.drop_redraw_inner();
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        self.drop.cost_ms = self.drop.cost_ms * 0.95 + ms * 0.05;
    }

    fn drop_redraw_inner(&mut self) {
        let Some(w) = self.wins.get_mut(&Kind::Vis) else { return };
        let d = &mut self.drop;
        let (fw, fh) = if d.fullscreen { (w.frame.width.max(1), w.frame.height.max(1)) } else { (d.width, d.height) };
        if !d.fullscreen {
            w.frame.resize(fw, fh);
            gen_frame(&mut w.frame, &self.skin, fw as i32, fh as i32, "G Drop", d.focused);
            if d.pressed_close {
                w.frame.sprite(&self.skin, "GEN_CLOSE_SELECTED", fw as i32 - 11, 3);
            }
            full_button(&mut w.frame, fw as i32 - 22, fw as i32 - 11);
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
        self.mark(Kind::Vis);
    }

    pub fn drop_press(&mut self, x: i32, y: i32) {
        let d = &self.drop;
        let (w, h) = (d.width as i32, d.height as i32);
        if d.fullscreen {
            if self.double_click(Kind::Vis, x, y) {
                self.drop_fullscreen(false);
            }
            return;
        }
        if (3..12).contains(&y) {
            if x >= w - 11 && x < w - 2 {
                self.drop.pressed_close = true;
                self.mark(Kind::Vis);
                return;
            }
            if x >= w - 22 && x < w - 13 {
                return self.drop_fullscreen(true);
            }
        }
        // Any edge resizes (the bottom-right grip too); the rest moves.
        const EDGE: i32 = 6;
        let grip = x >= w - 20 && y >= h - 20;
        let dir = (
            if x < EDGE { -1 } else if x >= w - EDGE || grip { 1 } else { 0 },
            if y < 3 { -1 } else if y >= h - EDGE || grip { 1 } else { 0 },
        );
        if dir != (0, 0) {
            if let Some(win) = self.win(Kind::Vis) {
                self.resize = Some(crate::app::Resize { start: win.global_cursor(), size: (self.drop.width, self.drop.height), dir, origin: win.pos });
            }
            return;
        }
        if self.double_click(Kind::Vis, x, y) {
            return self.drop_fullscreen(true);
        }
        self.start_drag(Kind::Vis);
    }

    /// A preset change from the menu or keys; shows its name.
    pub fn drop_step(&mut self, f: impl FnOnce(&mut gamp_vis::GDrop)) {
        f(&mut self.drop.engine);
        self.drop_changed();
    }

    pub fn drop_cycle(&mut self, on: bool) {
        self.drop.engine.auto = on;
        self.say(if on { "G-DROP: CYCLES PRESETS" } else { "G-DROP: STAYS ON THIS PRESET" });
    }

    /// Right-click in G-Drop.
    pub fn drop_menu(&self) -> Vec<Item> {
        let now = self.drop.engine.current();
        let presets = gamp_vis::PRESETS.iter().enumerate().map(|(i, p)| Item::action(format!("drop:preset:{i}"), p.name).checked(i == now)).collect();
        vec![
            Item::action("drop:full", if self.drop.fullscreen { "Leave full screen (Esc)" } else { "Full screen (F)" }),
            Item::Separator,
            Item::action("drop:next", "Next preset (Space)"),
            Item::action("drop:back", "Previous preset (←)"),
            Item::action("drop:random", "Random preset (H)"),
            Item::submenu("Presets", presets),
            Item::Separator,
            Item::action("drop:cycle", "Cycle presets (R)").checked(self.drop.engine.auto),
        ]
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
        let scale = w.scale() * self.zoom();
        let (dir, origin, size) = (r.dir, r.origin, r.size);
        let grow = |d: i8, moved: f64, from: u32| (from as f64 + d as f64 * moved / scale) as i32;
        let nw = grow(dir.0, now.0 - r.start.0, size.0).max(275) as u32;
        let nh = grow(dir.1, now.1 - r.start.1, size.1).max(116) as u32;
        let (nw, nh) = (275 + (nw - 275 + 12) / 25 * 25, 116 + (nh - 116 + 14) / 29 * 29);
        if (nw, nh) != (self.drop.width, self.drop.height) {
            self.drop.width = nw;
            self.drop.height = nh;
            // Dragging the left or top edge keeps the opposite one still.
            let shift = |d: i8, from: u32, to: u32| if d < 0 { ((from as f64 - to as f64) * scale).round() as i32 } else { 0 };
            let pos = (origin.0 + shift(dir.0, size.0, nw), origin.1 + shift(dir.1, size.1, nh));
            if let Some(w) = self.wins.get_mut(&Kind::Vis) {
                w.set_size(winit::dpi::LogicalSize::new(nw, nh));
                if dir.0 < 0 || dir.1 < 0 {
                    w.move_to(pos.0, pos.1);
                }
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
                "r" => self.drop_cycle(!self.drop.engine.auto),
                "f" => self.drop_fullscreen(!self.drop.fullscreen),
                _ => return false,
            },
            _ => return false,
        }
        self.drop_changed();
        true
    }
}

/// G-Drop's full-screen button at `x`, beside close (at `close`): the
/// close button's face with a box where its X is, in the skin's colors.
fn full_button(frame: &mut Frame, x: i32, close: i32) {
    let w = frame.width as i32;
    let at = |f: &Frame, dx: i32, dy: i32| f.pixels.get(((3 + dy) * w + close + dx) as usize).copied().unwrap_or(0);
    for dy in 0..9 {
        for dx in 0..9 {
            let c = at(frame, dx, dy);
            frame.put(x + dx, 3 + dy, c);
        }
    }
    let (face, mark) = (at(frame, 4, 1), at(frame, 4, 4));
    frame.fill(x + 2, 5, 5, 5, face);
    frame.fill(x + 2, 5, 5, 1, mark);
    frame.fill(x + 2, 9, 5, 1, mark);
    frame.fill(x + 2, 5, 1, 5, mark);
    frame.fill(x + 6, 5, 1, 5, mark);
}
