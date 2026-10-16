//! `gamp --selftest`: runs the real app without windows and checks that it
//! plays: a file straight through the engine, and a folder through the
//! library, the local server and the HTTP reader. For CI on every OS.

use std::time::{Duration, Instant};

use gamp_audio::OutputKind;
use gamp_ui::main_window::PlayState;
use serde_json::json;

use crate::app::App;
use crate::backend::Backend;

/// A 3-second mono tone as a WAV file.
pub fn tone(path: &std::path::Path) {
    let rate = 22050u32;
    let frames = rate * 3;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + frames * 2).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), rate.to_le_bytes().to_vec(), (rate * 2).to_le_bytes().to_vec(), 2u16.to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
        b.extend_from_slice(&v);
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(frames * 2).to_le_bytes());
    for i in 0..frames {
        b.extend_from_slice(&(((i as f32 * 0.05).sin() * 8000.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn wait(app: &mut App, secs: u64, what: &str, mut done: impl FnMut(&App) -> bool) -> anyhow::Result<()> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        app.tick();
        if done(app) {
            println!("ok: {what}");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    anyhow::bail!("timed out: {what} (state {:?}, status {:?})", app.main.state, app.lib.view.status)
}

pub fn run() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!("gamp-selftest-{}", std::process::id()));
    let music = root.join("Music");
    std::fs::create_dir_all(&music)?;
    tone(&music.join("Selftest Tone.wav"));
    if std::env::var_os("WINAMP_PORT").is_none() {
        // SAFETY: before any other thread reads the environment.
        unsafe { std::env::set_var("WINAMP_PORT", "8797") };
    }
    let backend = Backend::start(root.join("data"), None)?;
    println!("ok: local server on port {}", backend.port);
    let mut app = App::new(OutputKind::Null, Some(backend))?;

    app.open_files(vec![music.join("Selftest Tone.wav")], true);
    wait(&mut app, 10, "a file plays", |a| a.main.state == PlayState::Playing && a.main.elapsed > 0.3)?;
    app.player.stop();

    app.lib_start();
    let b = app.backend.as_ref().unwrap();
    b.post("/api/config", json!({ "folders": [music.to_string_lossy()] }), true);
    wait(&mut app, 10, "settings saved", |a| a.lib.config["folders"].as_array().is_some_and(|f| !f.is_empty()))?;
    app.backend.as_ref().unwrap().rescan();
    wait(&mut app, 20, "folder scanned", |a| a.lib.scan.as_ref().is_some_and(|s| !s.scanning && s.found == 1))?;
    app.lib_open(crate::library::Page::List { sid: "local".into(), node: "audio".into(), title: "Local".into() }, true);
    wait(&mut app, 10, "library lists the track", |a| !a.lib.items.is_empty())?;
    app.lib_send(app.lib.items.clone(), false);
    wait(&mut app, 10, "it plays through the local server", |a| a.main.state == PlayState::Playing && a.main.elapsed > 0.3)?;
    app.player.seek(2.0);
    wait(&mut app, 5, "seeking over HTTP", |a| a.main.elapsed >= 2.0)?;

    // Online, when asked: a pasted SoundCloud link plays and seeks.
    if let Ok(link) = std::env::var("GAMP_SELFTEST_LINK") {
        app.lib_open(crate::library::Page::Search { sid: "soundcloud".into(), title: "SoundCloud › Search".into() }, true);
        app.lib.view.search.value = link;
        app.lib_submit("search");
        wait(&mut app, 30, "a SoundCloud link plays", |a| a.main.state == PlayState::Playing && a.main.elapsed > 1.0)?;
        app.player.seek(120.0);
        wait(&mut app, 30, "seeking in a SoundCloud stream", |a| a.main.elapsed >= 120.0)?;
    }

    // Every window draws.
    for k in [crate::app::Kind::Main, crate::app::Kind::Eq, crate::app::Kind::Playlist, crate::app::Kind::Library, crate::app::Kind::Vis] {
        app.redraw(k);
    }
    println!("ok: every window draws");
    let _ = std::fs::remove_dir_all(&root);
    println!("selftest passed");
    Ok(())
}
