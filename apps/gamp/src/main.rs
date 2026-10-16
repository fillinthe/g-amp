//! G-Amp: the classic Winamp 2 player, rebuilt natively in Rust.
//!
//! Each Winamp window is its own borderless OS window, drawn pixel for
//! pixel by gamp-ui from a classic skin, and playback is gamp-audio's own
//! engine. Nothing here runs in a web view.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod playlist;

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gamp_audio::analysis::{self, Spectrum};
use gamp_audio::{Event as AudioEvent, Input, OutputKind, Player, State};
use gamp_skin::Skin;
use gamp_ui::Frame;
use gamp_ui::main_window::{self, Control, PlayState, View};
use gamp_ui::vis::VisMode;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use playlist::{Entry, Playlist};

const TICK: Duration = Duration::from_millis(33);
const MARQUEE_STEP: Duration = Duration::from_millis(220);

struct Surface {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

struct App {
    skin: Skin,
    player: Player,
    playlist: Playlist,
    /// Player track id → playlist index.
    ids: HashMap<u64, usize>,
    view: View,
    frame: Frame,
    main: Option<Surface>,
    double: bool,
    on_top: bool,
    cursor: (i32, i32),
    modifiers: ModifiersState,
    last_click: Option<(Instant, Control)>,
    marquee_at: Instant,
    blink_at: Instant,
    spectrum: Spectrum,
    wave: Vec<f32>,
    spec: Vec<f32>,
    bars: Vec<f32>,
    dirty: bool,
}

impl App {
    fn new(output: OutputKind) -> anyhow::Result<App> {
        let player = Player::new(output).or_else(|e| {
            log::warn!("no sound output ({e}); playing silently");
            Player::new(OutputKind::Null)
        })?;
        let mut view = View { volume: 0.8, focused: true, vis_mode: VisMode::Analyzer, ..Default::default() };
        view.title = "G-AMP".into();
        player.set_volume(view.volume);
        Ok(App {
            skin: Skin::default_skin(),
            player,
            playlist: Playlist::default(),
            ids: HashMap::new(),
            view,
            frame: Frame::new(main_window::WIDTH, main_window::HEIGHT),
            main: None,
            double: false,
            on_top: false,
            cursor: (0, 0),
            modifiers: ModifiersState::empty(),
            last_click: None,
            marquee_at: Instant::now(),
            blink_at: Instant::now(),
            spectrum: Spectrum::new(512),
            wave: vec![0.0; 512],
            spec: Vec::new(),
            bars: Vec::new(),
            dirty: true,
        })
    }

    // ---- window ---------------------------------------------------------

    fn logical_size(&self) -> LogicalSize<u32> {
        let k = if self.double { 2 } else { 1 };
        let h = if self.view.shade { main_window::SHADE_HEIGHT } else { main_window::HEIGHT };
        LogicalSize::new(main_window::WIDTH * k, h * k)
    }

    fn create_main(&mut self, el: &ActiveEventLoop) -> anyhow::Result<()> {
        let attrs = Window::default_attributes()
            .with_title("G-Amp")
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(self.logical_size());
        let window = Rc::new(el.create_window(attrs)?);
        let context = softbuffer::Context::new(window.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.main = Some(Surface { window, surface });
        Ok(())
    }

    fn resize(&mut self) {
        let size = self.logical_size();
        if let Some(m) = &self.main {
            let _ = m.window.request_inner_size(size);
        }
        self.dirty = true;
    }

    /// Scales the 1× frame up to the window's pixels (nearest neighbour,
    /// so skins stay sharp at any DPI and in double size).
    fn present(&mut self) {
        let shade = self.view.shade;
        let _ = shade;
        main_window::draw(&mut self.frame, &self.skin, &self.view);
        let Some(m) = self.main.as_mut() else { return };
        let size = m.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if m.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = m.surface.buffer_mut() else { return };
        let (fw, fh) = (self.frame.width, self.frame.height);
        let (bw, bh) = (size.width, size.height);
        for y in 0..bh {
            let sy = (y as u64 * fh as u64 / bh as u64) as u32;
            let src = &self.frame.pixels[(sy * fw) as usize..((sy + 1) * fw) as usize];
            let dst = &mut buf[(y * bw) as usize..((y + 1) * bw) as usize];
            for (x, d) in dst.iter_mut().enumerate() {
                *d = src[(x as u64 * fw as u64 / bw as u64) as usize] & 0x00FF_FFFF;
            }
        }
        let _ = buf.present();
        self.dirty = false;
    }

    fn to_frame(&self, x: f64, y: f64) -> (i32, i32) {
        let Some(m) = &self.main else { return (0, 0) };
        let size = m.window.inner_size();
        let fx = x * self.frame.width as f64 / size.width.max(1) as f64;
        let fy = y * self.frame.height as f64 / size.height.max(1) as f64;
        (fx as i32, fy as i32)
    }

    // ---- playback -------------------------------------------------------

    fn load_current(&mut self, play: bool) {
        let Some(entry) = self.playlist.current().cloned() else { return };
        match Input::file(&entry.path) {
            Ok(input) => {
                let id = self.player.load(input, play);
                self.ids.insert(id, self.playlist.current);
                self.queue_after(self.playlist.current);
            }
            Err(e) => self.view.message = Some(format!("CAN'T OPEN: {e}")),
        }
        self.update_title();
    }

    /// Lines up the next track so it follows with no gap.
    fn queue_after(&mut self, index: usize) {
        let Some(next) = self.playlist.after(index) else { return };
        let Some(entry) = self.playlist.entries.get(next).cloned() else { return };
        if let Ok(input) = Input::file(&entry.path) {
            let id = self.player.queue(input);
            self.ids.insert(id, next);
        }
    }

    fn update_title(&mut self) {
        self.view.title = match self.playlist.current() {
            Some(e) => {
                let len = e.duration.map(|d| format!(" ({}:{:02})", d as u64 / 60, d as u64 % 60)).unwrap_or_default();
                format!("{}. {}{}", self.playlist.current + 1, e.display(), len)
            }
            None => "G-AMP".into(),
        };
        self.view.scroll = 0;
        self.dirty = true;
    }

    fn open_files(&mut self, paths: Vec<PathBuf>, replace: bool) {
        // A skin dropped on the player becomes the skin.
        if let Some(skin) = paths.iter().find(|p| p.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "wsz" | "zip"))) {
            match std::fs::read(skin).map_err(anyhow::Error::from).and_then(|b| Skin::from_wsz(&b, &Skin::default_skin())) {
                Ok(s) => {
                    self.skin = s;
                    self.dirty = true;
                }
                Err(e) => self.view.message = Some(format!("NOT A SKIN: {e}")),
            }
            return;
        }
        let entries: Vec<Entry> = playlist::expand(&paths).iter().map(|p| Entry::read(p)).collect();
        if entries.is_empty() {
            return;
        }
        if replace || self.playlist.entries.is_empty() {
            self.playlist.set(entries);
            self.load_current(true);
        } else {
            self.playlist.append(entries);
        }
    }

    fn eject(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Open files")
            .add_filter("Audio", playlist::AUDIO)
            .add_filter("Winamp skins", &["wsz", "zip"])
            .pick_files();
        if let Some(files) = files {
            self.open_files(files, true);
        }
    }

    fn play(&mut self) {
        match self.player.status().state {
            State::Paused => self.player.play(),
            State::Playing => self.player.seek(0.0),
            State::Stopped => {
                if self.playlist.entries.is_empty() {
                    self.eject();
                } else if self.player.status().track.is_some() {
                    self.player.play();
                } else {
                    self.load_current(true);
                }
            }
        }
    }

    fn step(&mut self, forward: bool) {
        let moved = if forward { self.playlist.next() } else { self.playlist.previous() };
        if moved {
            let playing = self.player.status().state == State::Playing;
            self.load_current(playing || self.view.state != PlayState::Stopped);
        }
    }

    fn seek_by(&mut self, secs: f64) {
        let s = self.player.status();
        self.player.seek((s.position + secs).max(0.0));
    }

    fn set_volume(&mut self, v: f32) {
        self.view.volume = v.clamp(0.0, 1.0);
        self.player.set_volume(self.view.volume);
        self.view.message = Some(format!("VOLUME: {}%", (self.view.volume * 100.0).round()));
        self.dirty = true;
    }

    fn set_balance(&mut self, b: f32) {
        self.view.balance = b;
        self.player.set_balance(b);
        let pct = (b.abs() * 100.0).round();
        self.view.message = Some(if pct == 0.0 { "BALANCE: CENTER".into() } else { format!("BALANCE: {pct}% {}", if b < 0.0 { "LEFT" } else { "RIGHT" }) });
        self.dirty = true;
    }

    fn poll_audio(&mut self) {
        while let Ok(e) = self.player.events().try_recv() {
            match e {
                AudioEvent::Started { id, format } => {
                    if let Some(&i) = self.ids.get(&id) {
                        self.playlist.current = i;
                        self.update_title();
                        self.queue_after(i);
                    }
                    self.view.kbps = format.kbps;
                    self.view.khz = Some((format.rate as f32 / 1000.0).round() as u32);
                    self.view.stereo = Some(format.channels >= 2);
                    self.view.duration = format.duration;
                }
                AudioEvent::Finished { .. } => {}
                AudioEvent::Error { id, message } => {
                    log::warn!("playback: {message}");
                    // Skip a file that won't play.
                    if self.ids.get(&id) == Some(&self.playlist.current) && self.playlist.next() {
                        self.load_current(true);
                    }
                }
            }
            self.dirty = true;
        }
        let s = self.player.status();
        let state = match s.state {
            State::Playing => PlayState::Playing,
            State::Paused => PlayState::Paused,
            State::Stopped => PlayState::Stopped,
        };
        if state != self.view.state || (s.position - self.view.elapsed).abs() > 0.2 {
            self.dirty = true;
        }
        self.view.state = state;
        self.view.elapsed = s.position;
        if let Some(f) = s.format {
            self.view.duration = f.duration;
        }
    }

    fn tick(&mut self) {
        self.poll_audio();
        let now = Instant::now();
        // Visualizer.
        if self.view.state == PlayState::Playing && self.view.vis_mode != VisMode::Off {
            self.player.waveform(&mut self.wave);
            match self.view.vis_mode {
                VisMode::Analyzer => {
                    self.spectrum.compute(&self.wave, &mut self.spec);
                    analysis::bars(&self.spec, self.player.output_rate(), 19, &mut self.bars);
                    self.view.analyzer.update(&self.bars);
                }
                _ => {
                    self.view.wave.clear();
                    self.view.wave.extend_from_slice(&self.wave[self.wave.len() - 256..]);
                }
            }
            self.dirty = true;
        } else if self.view.analyzer.peaks.iter().any(|p| *p > 0.0) {
            self.view.analyzer.decay();
            self.dirty = true;
        }
        // Marquee.
        if now.duration_since(self.marquee_at) >= MARQUEE_STEP {
            self.marquee_at = now;
            if self.view.title.chars().count() * 5 > 154 && self.view.message.is_none() {
                self.view.scroll += 5;
                self.dirty = true;
            }
        }
        // Paused time blinks.
        if now.duration_since(self.blink_at) >= Duration::from_millis(1000) {
            self.blink_at = now;
            self.view.blink_on = !self.view.blink_on;
            if self.view.state == PlayState::Paused {
                self.dirty = true;
            }
        }
    }

    // ---- input ----------------------------------------------------------

    fn press(&mut self) {
        let (x, y) = self.cursor;
        let Some(c) = main_window::hit(x, y, self.view.shade) else { return };
        // Double-clicking the title bar rolls the window up, as in Winamp.
        if c == Control::TitleBar {
            let double = self.last_click.is_some_and(|(t, lc)| lc == Control::TitleBar && t.elapsed() < Duration::from_millis(400));
            self.last_click = Some((Instant::now(), c));
            if double {
                self.toggle_shade();
                return;
            }
            if let Some(m) = &self.main {
                let _ = m.window.drag_window();
            }
            return;
        }
        self.last_click = Some((Instant::now(), c));
        self.view.pressed = Some(c);
        self.drag();
        self.dirty = true;
    }

    /// Sliders follow the pointer while held.
    fn drag(&mut self) {
        let (x, _) = self.cursor;
        match self.view.pressed {
            Some(Control::Volume) => self.set_volume(main_window::volume_at(x)),
            Some(Control::Balance) => self.set_balance(main_window::balance_at(x)),
            Some(Control::Position) if self.view.duration.is_some() => {
                let p = main_window::position_at(x, self.view.shade);
                self.view.seeking = Some(p);
                let d = self.view.duration.unwrap_or(0.0);
                let t = d * p as f64;
                self.view.message = Some(format!("SEEK TO: {}:{:02}/{}:{:02} ({}%)", t as u64 / 60, t as u64 % 60, d as u64 / 60, d as u64 % 60, (p * 100.0).round()));
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Returns true when the app should quit.
    fn release(&mut self) -> bool {
        let Some(c) = self.view.pressed.take() else { return false };
        let (x, y) = self.cursor;
        let over = main_window::hit(x, y, self.view.shade) == Some(c);
        self.view.message = None;
        self.dirty = true;
        match c {
            Control::Position => {
                if let (Some(p), Some(d)) = (self.view.seeking.take(), self.view.duration) {
                    self.player.seek(d * p as f64);
                }
            }
            Control::Volume | Control::Balance => {}
            _ if !over => {}
            Control::Previous => self.step(false),
            Control::Next => self.step(true),
            Control::Play => self.play(),
            Control::Pause => match self.player.status().state {
                State::Playing => self.player.pause(),
                State::Paused => self.player.play(),
                State::Stopped => {}
            },
            Control::Stop => self.player.stop(),
            Control::Eject => self.eject(),
            Control::Shuffle => {
                self.view.shuffle = !self.view.shuffle;
                self.playlist.shuffle = self.view.shuffle;
                self.playlist.reshuffle();
            }
            Control::Repeat => {
                self.view.repeat = !self.view.repeat;
                self.playlist.repeat = self.view.repeat;
            }
            Control::Equalizer => self.view.eq_open = !self.view.eq_open,
            Control::Playlist => self.view.playlist_open = !self.view.playlist_open,
            Control::Time => self.view.show_remaining = !self.view.show_remaining,
            Control::Vis => {
                self.view.vis_mode = match self.view.vis_mode {
                    VisMode::Analyzer => VisMode::Oscilloscope,
                    VisMode::Oscilloscope => VisMode::Off,
                    VisMode::Off => VisMode::Analyzer,
                }
            }
            Control::Close => return true,
            Control::Minimize => {
                if let Some(m) = &self.main {
                    m.window.set_minimized(true);
                }
            }
            Control::Shade => self.toggle_shade(),
            Control::Clutter('d') => {
                self.double = !self.double;
                self.resize();
            }
            Control::Clutter('a') => self.toggle_on_top(),
            _ => {}
        }
        false
    }

    fn toggle_shade(&mut self) {
        self.view.shade = !self.view.shade;
        self.resize();
    }

    fn toggle_on_top(&mut self) {
        self.on_top = !self.on_top;
        if let Some(m) = &self.main {
            m.window.set_window_level(if self.on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal });
        }
    }

    /// Winamp's keys.
    fn key(&mut self, key: &Key) {
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        match key {
            Key::Character(c) => match c.to_lowercase().as_str() {
                "z" => self.step(false),
                "x" => self.play(),
                "c" => match self.player.status().state {
                    State::Playing => self.player.pause(),
                    State::Paused => self.player.play(),
                    _ => {}
                },
                "v" => self.player.stop(),
                "b" => self.step(true),
                "l" => self.eject(),
                "s" => self.release_like(Control::Shuffle),
                "r" => self.release_like(Control::Repeat),
                "d" if ctrl => {
                    self.double = !self.double;
                    self.resize();
                }
                "w" if self.modifiers.alt_key() => {}
                _ => {}
            },
            Key::Named(NamedKey::ArrowLeft) => self.seek_by(-5.0),
            Key::Named(NamedKey::ArrowRight) => self.seek_by(5.0),
            Key::Named(NamedKey::ArrowUp) => {
                self.set_volume(self.view.volume + 0.02);
                self.view.message = None;
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.set_volume(self.view.volume - 0.02);
                self.view.message = None;
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn release_like(&mut self, c: Control) {
        match c {
            Control::Shuffle => {
                self.view.shuffle = !self.view.shuffle;
                self.playlist.shuffle = self.view.shuffle;
                self.playlist.reshuffle();
            }
            Control::Repeat => {
                self.view.repeat = !self.view.repeat;
                self.playlist.repeat = self.view.repeat;
            }
            _ => {}
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.main.is_none() {
            if let Err(e) = self.create_main(el) {
                eprintln!("G-Amp can't open its window: {e}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.present(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.dirty = true,
            WindowEvent::Focused(f) => {
                self.view.focused = f;
                self.dirty = true;
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = self.to_frame(position.x, position.y);
                if self.view.pressed.is_some() {
                    self.drag();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.press(),
                ElementState::Released => {
                    if self.release() {
                        el.exit();
                    }
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y / 20.0,
                };
                self.set_volume(self.view.volume + (dy * 0.02) as f32);
                self.view.message = None;
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => self.key(&event.logical_key),
            WindowEvent::DroppedFile(path) => self.open_files(vec![path], false),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.tick();
        if self.dirty {
            if let Some(m) = &self.main {
                m.window.request_redraw();
            }
        }
        // Wake often only while something moves; idle is idle.
        let busy = self.view.state == PlayState::Playing || self.view.pressed.is_some() || self.view.analyzer.peaks.iter().any(|p| *p > 0.0) || self.view.title.chars().count() * 5 > 154 || self.view.state == PlayState::Paused;
        el.set_control_flow(if busy { ControlFlow::WaitUntil(Instant::now() + TICK) } else { ControlFlow::Wait });
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut app = App::new(OutputKind::Device)?;
    let files: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if !files.is_empty() {
        app.open_files(files, true);
    }
    let el = EventLoop::new()?;
    el.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(path: &std::path::Path, secs: u32) {
        let rate = 22050u32;
        let frames = secs * rate;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(frames * 2).to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 * 0.06).sin() * 9000.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    /// A click at window coordinates.
    fn click(app: &mut App, x: i32, y: i32) -> bool {
        app.cursor = (x, y);
        app.press();
        app.release()
    }

    fn settle(app: &mut App) {
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(25));
            app.tick();
        }
    }

    #[test]
    fn the_buttons_drive_the_player() {
        let dir = std::env::temp_dir().join(format!("gamp-app-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["01 One.wav", "02 Two.wav", "03 Three.wav"] {
            tone(&dir.join(n), 4);
        }
        let mut app = App::new(OutputKind::Null).unwrap();
        app.open_files(vec![dir.clone()], true);
        settle(&mut app);
        assert_eq!(app.playlist.entries.len(), 3);
        assert_eq!(app.view.state, PlayState::Playing);
        assert!(app.view.title.starts_with("1. 01 One"), "{}", app.view.title);
        assert_eq!(app.view.khz, Some(22));
        assert_eq!(app.view.stereo, Some(false));

        // Pause (62,88) holds; play (39,88) resumes.
        click(&mut app, 70, 95);
        settle(&mut app);
        assert_eq!(app.view.state, PlayState::Paused);
        click(&mut app, 45, 95);
        settle(&mut app);
        assert_eq!(app.view.state, PlayState::Playing);

        // Next (108,88) and previous (16,88).
        click(&mut app, 112, 95);
        settle(&mut app);
        assert_eq!(app.playlist.current, 1);
        assert!(app.view.title.starts_with("2. "));
        click(&mut app, 20, 95);
        settle(&mut app);
        assert_eq!(app.playlist.current, 0);

        // Volume slider: drag to the far left is silence.
        app.cursor = (110, 63);
        app.press();
        app.cursor = (100, 63);
        app.drag();
        assert_eq!(app.player.volume(), 0.0);
        assert!(app.view.message.as_deref().unwrap_or("").starts_with("VOLUME: 0%"));
        app.release();
        assert!(app.view.message.is_none());

        // Seek by the position bar to the middle.
        app.cursor = (16 + 14 + 110, 76);
        app.press();
        app.release();
        settle(&mut app);
        assert!(app.view.elapsed > 1.6 && app.view.elapsed < 2.6, "{}", app.view.elapsed);

        // Shuffle and repeat light up.
        click(&mut app, 180, 95);
        click(&mut app, 220, 95);
        assert!(app.view.shuffle && app.view.repeat);

        // Stop (85,88).
        click(&mut app, 90, 95);
        settle(&mut app);
        assert_eq!(app.view.state, PlayState::Stopped);

        // The title bar's close button quits.
        assert!(click(&mut app, 267, 6));

        // And the drawing of every state works.
        let mut f = Frame::new(1, 1);
        main_window::draw(&mut f, &app.skin, &app.view);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gapless_runs_through_the_list() {
        let dir = std::env::temp_dir().join(format!("gamp-app-gapless-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        tone(&dir.join("a.wav"), 1);
        tone(&dir.join("b.wav"), 1);
        let mut app = App::new(OutputKind::Null).unwrap();
        app.open_files(vec![dir.join("a.wav"), dir.join("b.wav")], true);
        let start = Instant::now();
        let mut saw_second = false;
        while start.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(30));
            app.tick();
            saw_second |= app.playlist.current == 1 && app.view.state == PlayState::Playing;
            if saw_second && app.view.state == PlayState::Stopped {
                break;
            }
        }
        assert!(saw_second);
        assert_eq!(app.view.state, PlayState::Stopped);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
