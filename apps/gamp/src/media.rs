//! Media keys and the system's Now Playing (Control Center and AirPods on
//! macOS, the media overlay on Windows, MPRIS on Linux).

use std::sync::mpsc;
use std::time::Duration;

use raw_window_handle::HasWindowHandle;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};

use crate::app::{App, Kind};

pub struct Media {
    controls: MediaControls,
    rx: mpsc::Receiver<MediaControlEvent>,
    shown: (String, u8),
}

impl Media {
    pub fn start(window: Option<&winit::window::Window>) -> Option<Media> {
        #[allow(unused_mut)]
        let mut hwnd = None;
        #[cfg(windows)]
        if let Some(w) = window {
            if let Ok(h) = w.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(h) = h.as_raw() {
                    hwnd = Some(h.hwnd.get() as *mut std::ffi::c_void);
                }
            }
        }
        let _ = window.map(|w| w.window_handle().is_ok());
        let config = PlatformConfig { display_name: "G-Amp", dbus_name: "co.fillinthe.gamp", hwnd };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                log::info!("media keys unavailable: {e:?}");
                return None;
            }
        };
        let (tx, rx) = mpsc::channel();
        if let Err(e) = controls.attach(move |e| {
            let _ = tx.send(e);
        }) {
            log::info!("media keys unavailable: {e:?}");
            return None;
        }
        Some(Media { controls, rx, shown: (String::new(), 255) })
    }
}

impl App {
    /// Handles media keys and keeps Now Playing up to date.
    pub fn media_tick(&mut self) {
        let Some(m) = self.media.as_mut() else { return };
        let events: Vec<MediaControlEvent> = m.rx.try_iter().collect();
        for e in events {
            match e {
                MediaControlEvent::Play => self.player.play(),
                MediaControlEvent::Pause => self.player.pause(),
                MediaControlEvent::Toggle => self.media_toggle(),
                MediaControlEvent::Next => self.menu_action("play:next", None),
                MediaControlEvent::Previous => self.menu_action("play:prev", None),
                MediaControlEvent::Stop => self.player.stop(),
                MediaControlEvent::Seek(SeekDirection::Forward) => self.menu_action("play:fwd", None),
                MediaControlEvent::Seek(SeekDirection::Backward) => self.menu_action("play:back", None),
                MediaControlEvent::SeekBy(dir, d) => {
                    let s = d.as_secs_f64() * if matches!(dir, SeekDirection::Forward) { 1.0 } else { -1.0 };
                    let at = self.player.status().position;
                    self.player.seek((at + s).max(0.0));
                }
                MediaControlEvent::SetPosition(MediaPosition(p)) => self.player.seek(p.as_secs_f64()),
                MediaControlEvent::SetVolume(v) => self.player.set_volume(v as f32),
                MediaControlEvent::Raise => {
                    if let Some(w) = self.win(Kind::Main).and_then(|w| w.window.clone()) {
                        w.focus_window();
                    }
                }
                _ => {}
            }
        }
        let Some(m) = self.media.as_mut() else { return };
        let entry = self.playlist.current().cloned();
        let title = entry.as_ref().map(|e| e.display()).unwrap_or_default();
        let state = self.main.state as u8;
        // Metadata when the song changes; position now and then.
        if m.shown.0 != title {
            if let Some(e) = &entry {
                let _ = m.controls.set_metadata(MediaMetadata {
                    title: Some(&e.title),
                    artist: (!e.artist.is_empty()).then_some(e.artist.as_str()),
                    album: None,
                    duration: e.duration.map(Duration::from_secs_f64),
                    cover_url: None,
                });
            }
        }
        if m.shown != (title.clone(), state) || self.main.state == gamp_ui::main_window::PlayState::Playing && self.media_at.elapsed() > Duration::from_secs(5) {
            let progress = Some(MediaPosition(Duration::from_secs_f64(self.main.elapsed.max(0.0))));
            let playback = match self.main.state {
                gamp_ui::main_window::PlayState::Playing => MediaPlayback::Playing { progress },
                gamp_ui::main_window::PlayState::Paused => MediaPlayback::Paused { progress },
                gamp_ui::main_window::PlayState::Stopped => MediaPlayback::Stopped,
            };
            let _ = m.controls.set_playback(playback);
            m.shown = (title, state);
            self.media_at = std::time::Instant::now();
        }
    }

    fn media_toggle(&mut self) {
        match self.player.status().state {
            gamp_audio::State::Playing => self.player.pause(),
            gamp_audio::State::Paused => self.player.play(),
            gamp_audio::State::Stopped => self.menu_action("play:play", None),
        }
    }
}
