//! Classic Winamp 2 skins (`.wsz`, a zip of bitmaps and text files) for
//! G-Amp's native windows.
//!
//! A skin is a set of sprite sheets (MAIN.BMP, CBUTTONS.BMP, TEXT.BMP...)
//! plus colours (PLEDIT.TXT, VISCOLOR.TXT) and window shapes (REGION.TXT).
//! Sprites are named the way Webamp names them (`MAIN_PLAY_BUTTON`...);
//! their rectangles are the Winamp 2 skin format, in `sprites.json`
//! (taken from Webamp, MIT licence). Anything a skin leaves out comes from
//! G-Amp's own default skin, as Winamp did with its base skin.

pub mod region;


use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use image::RgbaImage;
use serde::Deserialize;

/// G-Amp's own skin, "GAMP Classic" (made by `tools/skin-gen`).
pub static DEFAULT_SKIN: &[u8] = include_bytes!("../../../assets/skins/gamp-classic.wsz");

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize)]
struct SpriteDef {
    name: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Table {
    sprites: HashMap<String, Vec<SpriteDef>>,
    font: HashMap<String, [u32; 2]>,
}

/// sprite name → (sheet, rect).
pub fn sprite_table() -> &'static HashMap<String, (&'static str, Rect)> {
    static T: OnceLock<HashMap<String, (&'static str, Rect)>> = OnceLock::new();
    T.get_or_init(|| {
        let t: Table = serde_json::from_str(include_str!("sprites.json")).expect("sprites.json");
        let mut out = HashMap::new();
        for (sheet, list) in t.sprites {
            let sheet: &'static str = Box::leak(sheet.into_boxed_str());
            for s in list {
                out.insert(s.name, (sheet, Rect { x: s.x, y: s.y, width: s.width, height: s.height }));
            }
        }
        // TEXT.BMP characters: 5×6 cells.
        for (ch, [row, col]) in t.font {
            if let Some(c) = ch.chars().next() {
                let rect = Rect { x: col * 5, y: row * 6, width: 5, height: 6 };
                // Letters are stored lower case; look them up in either case.
                for c in [c, c.to_ascii_uppercase()] {
                    out.entry(format!("CHARACTER_{}", c as u32)).or_insert(("TEXT", rect));
                }
            }
        }
        out
    })
}

/// The sheets a skin can have, and their file names.
pub const SHEETS: &[(&str, &str)] = &[
    ("MAIN", "main"),
    ("CBUTTONS", "cbuttons"),
    ("TITLEBAR", "titlebar"),
    ("TEXT", "text"),
    ("NUMBERS", "numbers"),
    ("NUMS_EX", "nums_ex"),
    ("PLAYPAUS", "playpaus"),
    ("MONOSTER", "monoster"),
    ("POSBAR", "posbar"),
    ("VOLUME", "volume"),
    ("BALANCE", "balance"),
    ("SHUFREP", "shufrep"),
    ("EQMAIN", "eqmain"),
    ("EQ_EX", "eq_ex"),
    ("PLEDIT", "pledit"),
    ("GEN", "gen"),
];

/// The playlist's colours and font, from PLEDIT.TXT.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaylistStyle {
    pub normal: [u8; 3],
    pub current: [u8; 3],
    pub normal_bg: [u8; 3],
    pub selected_bg: [u8; 3],
    pub font: String,
}

impl Default for PlaylistStyle {
    fn default() -> Self {
        PlaylistStyle { normal: [0, 255, 0], current: [255, 255, 255], normal_bg: [0, 0, 0], selected_bg: [0, 0, 198], font: "Arial".into() }
    }
}

pub struct Skin {
    pub name: String,
    sheets: HashMap<&'static str, RgbaImage>,
    pub playlist: PlaylistStyle,
    /// 24 colours: background, dots, 16 analyzer bar shades (top to
    /// bottom), 5 oscilloscope shades, peak dots.
    pub viscolors: Vec<[u8; 3]>,
    pub regions: region::Regions,
}

fn parse_color(v: &str) -> Option<[u8; 3]> {
    let h = v.trim().trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let n = u32::from_str_radix(&h[..6], 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

pub fn parse_pledit(text: &str) -> PlaylistStyle {
    let mut s = PlaylistStyle::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "normal" => s.normal = parse_color(v).unwrap_or(s.normal),
            "current" => s.current = parse_color(v).unwrap_or(s.current),
            "normalbg" => s.normal_bg = parse_color(v).unwrap_or(s.normal_bg),
            "selectedbg" => s.selected_bg = parse_color(v).unwrap_or(s.selected_bg),
            "font" => s.font = v.trim().to_owned(),
            _ => {}
        }
    }
    s
}

/// VISCOLOR.TXT: one "r,g,b" per line, comments after.
pub fn parse_viscolor(text: &str) -> Vec<[u8; 3]> {
    text.lines()
        .filter_map(|l| {
            let nums: Vec<u8> = l.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(3).filter_map(|p| p.parse::<u32>().ok().map(|n| n.min(255) as u8)).collect();
            (nums.len() == 3).then(|| [nums[0], nums[1], nums[2]])
        })
        .take(24)
        .collect()
}

impl Skin {
    /// G-Amp's own skin.
    pub fn default_skin() -> Skin {
        Skin::from_wsz_inner(DEFAULT_SKIN, None).expect("the built-in skin loads")
    }

    /// A `.wsz` (or `.zip`) skin; what it lacks comes from `base`.
    pub fn from_wsz(bytes: &[u8], base: &Skin) -> anyhow::Result<Skin> {
        Skin::from_wsz_inner(bytes, Some(base))
    }

    fn from_wsz_inner(bytes: &[u8], base: Option<&Skin>) -> anyhow::Result<Skin> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        // Skins nest files in folders and use any case; match on the base name.
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name()?.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
            let mut data = Vec::new();
            f.read_to_end(&mut data)?;
            files.insert(name, data);
        }
        let mut sheets = HashMap::new();
        for (sheet, file) in SHEETS {
            let found = ["bmp", "png"].iter().find_map(|ext| files.get(&format!("{file}.{ext}")));
            if let Some(data) = found {
                match image::load_from_memory(data) {
                    Ok(img) => {
                        sheets.insert(*sheet, img.to_rgba8());
                    }
                    Err(e) => log::warn!("skin: can't read {file}: {e}"),
                }
            }
        }
        // Winamp's own fallbacks inside a skin.
        if !sheets.contains_key("BALANCE") {
            if let Some(v) = sheets.get("VOLUME").cloned() {
                sheets.insert("BALANCE", v);
            }
        }
        if let Some(base) = base {
            for (sheet, img) in &base.sheets {
                sheets.entry(sheet).or_insert_with(|| img.clone());
            }
        }
        let text = |name: &str| files.get(name).map(|b| String::from_utf8_lossy(b).into_owned());
        let playlist = text("pledit.txt").map(|t| parse_pledit(&t)).or_else(|| base.map(|b| b.playlist.clone())).unwrap_or_default();
        let mut viscolors = text("viscolor.txt").map(|t| parse_viscolor(&t)).unwrap_or_default();
        if viscolors.len() < 24 {
            let fallback = base.map(|b| b.viscolors.clone()).unwrap_or_else(default_viscolors);
            viscolors.extend_from_slice(&fallback[viscolors.len().min(fallback.len())..]);
        }
        let regions = text("region.txt").map(|t| region::parse(&t)).unwrap_or_default();
        Ok(Skin { name: String::new(), sheets, playlist, viscolors, regions })
    }

    /// The generic windows' title letters (GEN.BMP rows 88 and 96): each
    /// letter's rectangle, found the way Winamp does, by the background
    /// colour columns between letters. `active` picks the focused row.
    pub fn gen_letter(&self, c: char, active: bool) -> Option<(&RgbaImage, Rect)> {
        let img = self.sheets.get("GEN")?;
        let y = if active { 88 } else { 96 };
        if img.height() < y + 7 {
            return None;
        }
        let index = (c.to_ascii_uppercase() as u32).checked_sub('A' as u32).filter(|i| *i < 26)?;
        let bg = img.get_pixel(0, y);
        let mut x = 1;
        for i in 0..=index {
            let mut end = x;
            while end < img.width() && img.get_pixel(end, y) != bg {
                end += 1;
            }
            if i == index {
                return (end > x).then_some((img, Rect { x, y, width: end - x, height: 7 }));
            }
            x = end + 1;
        }
        None
    }

    pub fn has(&self, sheet: &str) -> bool {
        self.sheets.contains_key(sheet)
    }

    pub fn sheet(&self, sheet: &str) -> Option<&RgbaImage> {
        self.sheets.get(sheet)
    }

    /// A sprite's sheet and rectangle, if this skin has it (some skins'
    /// sheets are smaller than the format allows).
    pub fn sprite(&self, name: &str) -> Option<(&RgbaImage, Rect)> {
        let (sheet, rect) = sprite_table().get(name)?;
        let img = self.sheets.get(sheet)?;
        (rect.x + rect.width <= img.width() && rect.y + rect.height <= img.height()).then_some((img, *rect))
    }
}

/// Winamp's classic analyzer colours.
pub fn default_viscolors() -> Vec<[u8; 3]> {
    let mut v = vec![[0, 0, 0], [24, 33, 41]];
    let bars: [[u8; 3]; 16] = [
        [239, 49, 16], [206, 41, 16], [214, 90, 0], [214, 102, 0], [214, 115, 0], [198, 123, 8], [222, 165, 24], [214, 181, 33],
        [189, 222, 41], [148, 222, 33], [41, 206, 16], [50, 190, 16], [57, 181, 16], [49, 156, 8], [41, 148, 0], [24, 132, 8],
    ];
    v.extend_from_slice(&bars);
    v.extend_from_slice(&[[255, 255, 255], [214, 214, 222], [181, 189, 189], [160, 170, 175], [148, 156, 165], [150, 150, 150]]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_the_main_sprites() {
        let t = sprite_table();
        let (sheet, r) = t["MAIN_PLAY_BUTTON"];
        assert_eq!(sheet, "CBUTTONS");
        assert_eq!((r.width, r.height), (23, 18));
        assert_eq!(t["MAIN_WINDOW_BACKGROUND"].1.width, 275);
        assert_eq!(t["CHARACTER_65"].1.width, 5); // "A"
        assert!(t.len() > 300);
    }

    #[test]
    fn reads_pledit_and_viscolor() {
        let p = parse_pledit("[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000FF\nFont=Arial");
        assert_eq!(p.selected_bg, [0, 0, 255]);
        assert_eq!(p.font, "Arial");
        let v = parse_viscolor("0,0,0, // color 0 = black\n24,33,41 // grid\n");
        assert_eq!(v, vec![[0, 0, 0], [24, 33, 41]]);
    }

    #[test]
    fn the_default_skin_is_complete() {
        let s = Skin::default_skin();
        for (sheet, _) in SHEETS {
            assert!(s.has(sheet), "missing {sheet}");
        }
        assert_eq!(s.viscolors.len(), 24);
        let (_, a) = s.gen_letter('A', true).expect("GEN letters");
        assert_eq!((a.x, a.y, a.height), (1, 88, 7));
        assert!(s.gen_letter('Z', false).is_some());
        for name in ["MAIN_WINDOW_BACKGROUND", "MAIN_PLAY_BUTTON", "MAIN_VOLUME_THUMB", "EQ_WINDOW_BACKGROUND", "PLAYLIST_TOP_TILE", "GEN_TOP_LEFT", "CHARACTER_65", "DIGIT_0"] {
            assert!(s.sprite(name).is_some(), "missing sprite {name}");
        }
    }
}
