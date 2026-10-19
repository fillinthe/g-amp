//! Winamp's equalizer preset files: `.eqf` (one preset) and `.q1`
//! libraries (many), the same layout. A header, then for each preset a
//! 257-byte name and eleven bytes: the ten bands, then the preamp, each
//! a slider step from 0 (+12 dB) through 31 (0 dB) to 63 (-12 dB).

use std::path::{Path, PathBuf};

const HEADER: &[u8] = b"Winamp EQ library file v1.1\x1a!--";
const NAME: usize = 257;

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    pub preamp: f32,
    pub bands: [f32; 10],
}

fn db(b: u8) -> f32 {
    let b = b.min(63) as f32;
    if b <= 31.0 { (31.0 - b) / 31.0 * 12.0 } else { (31.0 - b) / 32.0 * 12.0 }
}

fn byte(db: f32) -> u8 {
    let db = db.clamp(-12.0, 12.0);
    (if db >= 0.0 { 31.0 - db / 12.0 * 31.0 } else { 31.0 - db / 12.0 * 32.0 }).round() as u8
}

pub fn parse(data: &[u8]) -> Vec<Preset> {
    let Some(mut rest) = data.strip_prefix(HEADER) else { return Vec::new() };
    let mut out = Vec::new();
    while rest.len() >= NAME + 11 {
        let (name, values) = rest[..NAME + 11].split_at(NAME);
        let name = String::from_utf8_lossy(&name[..name.iter().position(|b| *b == 0).unwrap_or(NAME)]).trim().to_owned();
        let mut bands = [0.0; 10];
        for (b, v) in bands.iter_mut().zip(values) {
            *b = db(*v);
        }
        out.push(Preset { name, preamp: db(values[10]), bands });
        rest = &rest[NAME + 11..];
    }
    out
}

pub fn write(p: &Preset) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    let mut name = [0u8; NAME];
    let n = p.name.as_bytes();
    // Room for the terminating zero; cut at a character boundary.
    let len = (0..=n.len().min(NAME - 1)).rev().find(|i| p.name.is_char_boundary(*i)).unwrap_or(0);
    name[..len].copy_from_slice(&n[..len]);
    out.extend_from_slice(&name);
    out.extend(p.bands.iter().map(|b| byte(*b)));
    out.push(byte(p.preamp));
    out
}

/// Presets saved in Gamp's own folder, by name.
pub fn saved(dir: &Path) -> Vec<(PathBuf, Preset)> {
    let mut out: Vec<(PathBuf, Preset)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("eqf")))
                .filter_map(|p| {
                    let mut preset = parse(&std::fs::read(&p).ok()?).into_iter().next()?;
                    // The file's name is what was typed in the save dialog.
                    preset.name = p.file_stem()?.to_string_lossy().into_owned();
                    Some((p, preset))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(_, p)| p.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_reads_libraries() {
        let p = Preset { name: "Mine".into(), preamp: -3.0, bands: [12.0, 6.0, 0.0, -6.0, -12.0, 3.0, 0.0, 0.0, 9.0, -9.0] };
        let file = write(&p);
        assert_eq!(file.len(), HEADER.len() + NAME + 11);
        let back = &parse(&file)[0];
        assert_eq!(back.name, "Mine");
        for (a, b) in back.bands.iter().chain([&back.preamp]).zip(p.bands.iter().chain([&p.preamp])) {
            assert!((a - b).abs() < 0.2, "{a} vs {b}");
        }
        // A .q1 library: the same entries, one after another.
        let mut lib = file.clone();
        lib.extend_from_slice(&file[HEADER.len()..]);
        assert_eq!(parse(&lib).len(), 2);
        assert!(parse(b"not a preset").is_empty());
        assert_eq!((db(0), db(31), db(63)), (12.0, 0.0, -12.0));
        assert_eq!((byte(12.0), byte(0.0), byte(-12.0)), (0, 31, 63));
        assert_eq!(write(&Preset { name: "é".repeat(200), ..p }).len(), file.len());
    }
}
