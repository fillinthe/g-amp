//! Gamp: the classic Winamp 2 player, rebuilt natively in Rust.
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
mod eqf;
mod info;
mod jump;
mod layout;
mod library;
mod media;
mod playlist;
mod selftest;
mod session;
mod update;
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
    /// G-Drop steps on its own, faster clock; the rest stays at TICK.
    next_drop: Instant,
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
            UserEvent::Open(files) => self.app.open_files(files, true),
            // A window off every screen comes back with the rest; otherwise
            // the windows just come forward.
            UserEvent::Reopen if self.app.window_lost() => self.app.gather(),
            UserEvent::Reopen => {
                for k in [Kind::Main, Kind::Eq, Kind::Playlist, Kind::Library, Kind::Vis] {
                    if let Some(w) = self.app.win(k).filter(|w| w.visible).and_then(|w| w.window.as_ref()) {
                        win::order_front(w, self.app.on_top);
                    }
                }
            }
        }
    }

    /// Quit from the Dock, the menu bar (⌘Q) or a shutdown: winit's own
    /// exit paths save already, this one didn't.
    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.app.save_session();
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            self.app.open_windows(el);
            // Did an update we quit for install? Is there a newer Gamp?
            // (Quietly; it says so if there is.)
            self.app.update_now = std::env::args().any(|a| a == "--update");
            if self.app.update_now {
                self.app.update_check(true);
            } else if std::env::var_os("GAMP_NO_UPDATE_CHECK").is_none() {
                if !self.app.update_after_restart() {
                    self.app.update_check(false);
                }
            }
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
                } else if k == Kind::Jump {
                    app.jump_show(false);
                } else if k == Kind::Info {
                    app.info_hide();
                }
            }
            WindowEvent::RedrawRequested => app.redraw(k),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => app.mark(k),
            WindowEvent::Moved(p) => app.moved(k, p.x, p.y),
            WindowEvent::Focused(f) => app.set_focus(k, f),
            WindowEvent::Occluded(o) => app.occluded(k, o),
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
            self.next_tick = now + TICK;
        }
        let drop = app.drop_animating();
        if drop && now >= self.next_drop {
            app.mark(Kind::Vis);
            self.next_drop = now + app.drop_tick();
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
        el.set_control_flow(if app.busy() { ControlFlow::WaitUntil(if drop { self.next_tick.min(self.next_drop) } else { self.next_tick }) } else { ControlFlow::Wait });
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,librespot=warn")).init();
    if std::env::args().any(|a| a == "--selftest") {
        return selftest::run();
    }
    if std::env::args().any(|a| a == "--version") {
        println!("Gamp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let el = EventLoop::<UserEvent>::with_user_event().build()?;
    #[cfg(target_os = "macos")]
    finder::listen(el.create_proxy());
    let data = app::data_dir();
    // The sources, sign-ins and streaming; Gamp still plays files without it.
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
    el.run_app(&mut Shell { app, started: false, next_tick: Instant::now(), next_drop: Instant::now() })?;
    Ok(())
}

/// The Finder hands files to a running (or starting) app as an "open
/// documents" message to its delegate, not as arguments. winit's delegate
/// doesn't answer it, so Gamp adds the method to winit's delegate class.
#[cfg(target_os = "macos")]
mod finder {
    use std::ffi::{c_char, CStr};
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{msg_send, sel};
    use winit::event_loop::EventLoopProxy;

    use crate::backend::UserEvent;

    static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();

    unsafe extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *mut AnyObject) {
        let Some(urls) = (unsafe { urls.as_ref() }) else { return };
        let count: usize = unsafe { msg_send![urls, count] };
        let files: Vec<PathBuf> = (0..count)
            .filter_map(|i| {
                let url: *mut AnyObject = unsafe { msg_send![urls, objectAtIndex: i] };
                let path: *mut AnyObject = unsafe { msg_send![url.as_ref()?, path] };
                let s: *const c_char = unsafe { msg_send![path.as_ref()?, UTF8String] };
                (!s.is_null()).then(|| PathBuf::from(unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()))
            })
            .collect();
        if let Some(p) = PROXY.get().filter(|_| !files.is_empty()) {
            let _ = p.send_event(UserEvent::Open(files));
        }
    }

    /// A Dock click on the running app.
    unsafe extern "C-unwind" fn reopen(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, _visible: Bool) -> Bool {
        if let Some(p) = PROXY.get() {
            let _ = p.send_event(UserEvent::Reopen);
        }
        Bool::YES
    }

    pub fn listen(proxy: EventLoopProxy<UserEvent>) {
        let _ = PROXY.set(proxy);
        let Some(class) = AnyClass::get(c"WinitApplicationDelegate") else {
            log::warn!("no app delegate: files opened from the Finder won't arrive");
            return;
        };
        let f: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) = open_urls;
        // application:openURLs: is (void)(id self, SEL _cmd, NSApplication*, NSArray<NSURL*>*).
        unsafe {
            objc2::ffi::class_addMethod(class as *const AnyClass as *mut AnyClass, sel!(application:openURLs:), std::mem::transmute::<_, Imp>(f), c"v@:@@".as_ptr());
            let r: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool = reopen;
            let types = if cfg!(target_arch = "aarch64") { c"B@:@B" } else { c"c@:@c" };
            objc2::ffi::class_addMethod(class as *const AnyClass as *mut AnyClass, sel!(applicationShouldHandleReopen:hasVisibleWindows:), std::mem::transmute::<_, Imp>(r), types.as_ptr());
        }
    }
}
