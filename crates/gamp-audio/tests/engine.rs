//! The whole engine, through the silent output.

use std::time::{Duration, Instant};

use gamp_audio::{Event, Input, OutputKind, Player, State};

/// A WAV tone of `secs` seconds at `rate`.
fn tone(path: &std::path::Path, secs: f32, rate: u32, freq: f32) {
    let frames = (secs * rate as f32) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + frames * 4).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(frames * 4).to_le_bytes());
    for i in 0..frames {
        let v = ((std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * 12000.0) as i16;
        b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn wait_for(p: &Player, secs: f32, mut want: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs_f32(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match p.events().recv_timeout(left) {
            Ok(e) if want(&e) => return e,
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for an event"),
        }
    }
}

#[test]
fn plays_seeks_and_continues_gaplessly() {
    let dir = std::env::temp_dir().join(format!("gamp-audio-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    tone(&a, 3.0, 44100, 440.0);
    tone(&b, 1.0, 22050, 880.0);

    let p = Player::new(OutputKind::Null).unwrap();
    let first = p.load(Input::file(&a).unwrap(), true);
    let e = wait_for(&p, 3.0, |e| matches!(e, Event::Started { .. }));
    match e {
        Event::Started { id, format } => {
            assert_eq!(id, first);
            assert_eq!(format.rate, 44100);
            assert!((format.duration.unwrap() - 3.0).abs() < 0.01);
            assert!(format.seekable);
        }
        _ => unreachable!(),
    }
    std::thread::sleep(Duration::from_millis(400));
    let s = p.status();
    assert_eq!(s.state, State::Playing);
    // Moving forward (busy CI machines run the silent output slowly).
    assert!(s.position > 0.02 && s.position < 1.5, "{}", s.position);

    // The visualizers hear the tone.
    let mut wave = vec![0f32; 512];
    p.waveform(&mut wave);
    assert!(wave.iter().any(|v| v.abs() > 0.1));

    // Seek near the end; position follows.
    p.seek(2.5);
    std::thread::sleep(Duration::from_millis(150));
    let s = p.status();
    assert!(s.position >= 2.5 && s.position < 3.0, "{}", s.position);

    // Queue the next one: it starts with no gap, at its own rate.
    let second = p.queue(Input::file(&b).unwrap());
    let e = wait_for(&p, 3.0, |e| matches!(e, Event::Started { .. }));
    assert!(matches!(e, Event::Started { id, .. } if id == second));
    // Then everything ends.
    wait_for(&p, 3.0, |e| matches!(e, Event::Finished { id } if *id == second));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(p.status().state, State::Stopped);

    // Pause holds the position.
    p.load(Input::file(&a).unwrap(), true);
    std::thread::sleep(Duration::from_millis(300));
    p.pause();
    std::thread::sleep(Duration::from_millis(100));
    let at = p.status().position;
    std::thread::sleep(Duration::from_millis(300));
    assert!((p.status().position - at).abs() < 0.02);
    assert_eq!(p.status().state, State::Paused);

    // A file that isn't audio is reported, not a crash.
    let junk = dir.join("junk.mp3");
    std::fs::write(&junk, b"not audio at all").unwrap();
    p.load(Input::file(&junk).unwrap(), true);
    wait_for(&p, 3.0, |e| matches!(e, Event::Error { .. }));
    let _ = std::fs::remove_dir_all(&dir);
}
