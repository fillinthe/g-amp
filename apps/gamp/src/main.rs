//! G-Amp: the classic Winamp 2 player, rebuilt natively in Rust.
//!
//! Each Winamp window is its own borderless OS window, drawn pixel for
//! pixel by gamp-ui from a classic skin, and playback is gamp-audio's own
//! engine. Nothing here runs in a web view.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod layout;
mod playlist;
mod win;

use std::path::PathBuf;
use std::time::Instant;

use gamp_audio::OutputKind;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use app::{App, Kind, TICK};

struct Shell {
    app: App,
    started: bool,
}

impl ApplicationHandler for Shell {
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
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => app.key(k, &event.logical_key),
            WindowEvent::DroppedFile(path) => app.dropped(k, path),
            _ => {}
        }
        if app.quit {
            el.exit();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let app = &mut self.app;
        app.flush_drops();
        app.tick();
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
            el.exit();
        }
        // Wake often only while something moves; idle is idle.
        el.set_control_flow(if app.busy() { ControlFlow::WaitUntil(Instant::now() + TICK) } else { ControlFlow::Wait });
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
    el.run_app(&mut Shell { app, started: false })?;
    Ok(())
}
