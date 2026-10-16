//! G-Amp: the classic Winamp 2 player, rebuilt natively in Rust.
//!
//! Each Winamp window is its own borderless OS window, drawn pixel for
//! pixel by gamp-ui from a classic skin, and playback is gamp-audio's own
//! engine. Nothing here runs in a web view.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backend;
mod clipboard;
mod drop;
mod layout;
mod library;
mod media;
mod playlist;
mod selftest;
mod session;
mod stream;
mod win;

use std::path::PathBuf;
use std::time::Instant;

use gamp_audio::OutputKind;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use app::{App, Kind, TICK};
use backend::{Backend, UserEvent};

struct Shell {
    app: App,
    started: bool,
    /// When animation next steps (marquee, visualizers, time).
    next_tick: Instant,
}

impl ApplicationHandler<UserEvent> for Shell {
    fn user_event(&mut self, _el: &ActiveEventLoop, event: UserEvent) {
        match event {
            // A sign-in finished in the browser: come forward, refresh.
            UserEvent::Focus => {
                if let Some(w) = self.app.win(Kind::Main).and_then(|w| w.window.clone()) {
                    w.focus_window();
                }
                if let Some(b) = &self.app.backend {
                    b.refresh_sources();
                    b.config();
                }
                self.app.lib_open(library::Page::Settings, true);
            }
            UserEvent::Wake => {}
        }
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            self.app.open_windows(el);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(k) = self.app.kind_of(id) else { return };
        let app = &mut self.app;
        match event {
            WindowEvent::CloseRequested => {
                if k == Kind::Main {
                    app.quit = true;
                } else if let Kind::Menu(_) = k {
                    app.close_menus();
                }
            }
            WindowEvent::RedrawRequested => app.redraw(k),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => app.mark(k),
            WindowEvent::Moved(p) => app.moved(k, p.x, p.y),
            WindowEvent::Focused(f) => app.set_focus(k, f),
            WindowEvent::ModifiersChanged(m) => app.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => app.cursor_moved(k, position.x, position.y, Some(el)),
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => app.press(k, Some(el)),
                (MouseButton::Left, ElementState::Released) => app.release(k, Some(el)),
                (MouseButton::Right, ElementState::Pressed) if !matches!(k, Kind::Menu(_)) => app.context_menu(k, Some(el)),
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 20.0,
                };
                app.wheel(k, lines);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => app.key(k, &event.logical_key, event.text.as_deref()),
            WindowEvent::DroppedFile(path) => app.dropped(k, path),
            _ => {}
        }
        if app.quit {
            app.save_session();
            el.exit();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let app = &mut self.app;
        app.flush_drops();
        // Animate on a clock, not on every event (redraws are events too).
        let now = Instant::now();
        if now >= self.next_tick {
            app.tick();
            let step = if app.drop_visible() { app.drop_tick() } else { TICK };
            self.next_tick = now + step;
        }
        for k in app.dirty_kinds() {
            match k {
                Kind::Menu(_) => app.redraw(k),
                _ => {
                    if let Some(w) = app.win(k) {
                        w.request_redraw();
                    }
                }
            }
        }
        if app.quit {
            app.save_session();
            el.exit();
        }
        if app.last_save.elapsed() > std::time::Duration::from_secs(30) {
            app.last_save = Instant::now();
            app.save_session();
        }
        // Wake often only while something moves; idle is idle.
        el.set_control_flow(if app.busy() { ControlFlow::WaitUntil(self.next_tick) } else { ControlFlow::Wait });
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,librespot=warn")).init();
    if std::env::args().any(|a| a == "--selftest") {
        return selftest::run();
    }
    if std::env::args().any(|a| a == "--version") {
        println!("G-Amp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let el = EventLoop::<UserEvent>::with_user_event().build()?;
    let data = app::data_dir();
    // The sources, sign-ins and streaming; G-Amp still plays files without it.
    let backend = match Backend::start(data, Some(el.create_proxy())) {
        Ok(b) => Some(b),
        Err(e) => {
            log::warn!("music sources unavailable: {e}");
            None
        }
    };
    let mut app = App::new(OutputKind::Device, backend)?;
    app.session = app.load_session();
    let files: Vec<PathBuf> = std::env::args().skip(1).filter(|a| !a.starts_with("--") && !a.starts_with("-psn_")).map(PathBuf::from).collect();
    if !files.is_empty() {
        app.open_files(files, true);
    }
    // Nothing to play yet: show where music comes from.
    app.lib_open_at_start = app.playlist.entries.is_empty() && app.session.is_none();
    el.run_app(&mut Shell { app, started: false, next_tick: Instant::now() })?;
    Ok(())
}
