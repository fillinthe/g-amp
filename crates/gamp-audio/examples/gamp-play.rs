//! Plays files through the engine: `gamp-play [--volume 0.5] [--eq b1,..,b10] file...`

use std::time::Duration;

use gamp_audio::{EqSettings, Event, Input, OutputKind, Player, State};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    let mut volume = 0.6;
    let mut eq = EqSettings::default();
    let mut files = Vec::new();
    let mut seconds: Option<f64> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--volume" => volume = args.next().unwrap_or_default().parse()?,
            "--seconds" => seconds = Some(args.next().unwrap_or_default().parse()?),
            "--eq" => {
                for (i, v) in args.next().unwrap_or_default().split(',').enumerate().take(10) {
                    eq.bands[i] = v.parse()?;
                }
            }
            _ => files.push(a),
        }
    }
    let p = Player::new(OutputKind::Device)?;
    println!("output: {} Hz", p.output_rate());
    p.set_volume(volume);
    p.set_eq(eq);
    for (i, f) in files.iter().enumerate() {
        let input = Input::file(std::path::Path::new(f))?;
        if i == 0 { p.load(input, true); } else { p.queue(input); }
    }
    let started = std::time::Instant::now();
    loop {
        while let Ok(e) = p.events().try_recv() {
            match e {
                Event::Started { format, .. } => println!("playing: {format:?}"),
                Event::Finished { .. } => println!("finished"),
                Event::Error { message, .. } => println!("error: {message}"),
            }
        }
        let s = p.status();
        if s.state == State::Stopped && started.elapsed() > Duration::from_millis(500) {
            break;
        }
        if seconds.is_some_and(|t| started.elapsed().as_secs_f64() > t) {
            println!("position after {:.1}s: {:.2}s", started.elapsed().as_secs_f64(), s.position);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
