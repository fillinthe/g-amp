//! Draws GAMP Classic, Gamp's default skin, as a standard Winamp 2 `.wsz`
//! (so it is also a valid skin for any Winamp-compatible player).
//!
//!     cargo run -p skin-gen            # writes assets/skins/gamp-classic.wsz
//!     cargo run -p skin-gen -- out/    # also writes each bitmap as PNG

mod draw;
mod font;

use std::io::Write;

use draw::{Canvas, Rgb, mix, text_width};

// ---- palette -----------------------------------------------------------

const BODY: Rgb = [46, 46, 72];
const BODY_HI: Rgb = [86, 86, 128];
const BODY_LO: Rgb = [16, 16, 26];
const BODY_DARK: Rgb = [30, 30, 48];
const LCD: Rgb = [0, 0, 0];
const LCD_EDGE_HI: Rgb = [70, 70, 104];
const GREEN: Rgb = [44, 236, 60];
const GREEN_DIM: Rgb = [18, 54, 24];
const AMBER: Rgb = [255, 178, 30];
const AMBER_HI: Rgb = [255, 226, 120];
const AMBER_LO: Rgb = [176, 96, 10];
const GREY_LINE: Rgb = [110, 110, 140];
const GREY_LINE_LO: Rgb = [58, 58, 84];
const BTN: Rgb = [70, 70, 106];
const BTN_HI: Rgb = [128, 128, 176];
const BTN_LO: Rgb = [14, 14, 22];
const BTN_DOWN: Rgb = [44, 44, 68];
const ICON: Rgb = [232, 232, 248];
const LABEL: Rgb = [176, 176, 204];

// ---- shared pieces -------------------------------------------------------

/// A raised button face.
fn button(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, down: bool) {
    c.rect(x, y, w, h, if down { BTN_DOWN } else { BTN });
    c.vgrad(x + 1, y + 1, w - 2, h / 2, if down { BTN_DOWN } else { mix(BTN, BTN_HI, 0.35) }, if down { BTN_DOWN } else { BTN });
    if down {
        c.sunken(x, y, w, h, BTN_HI, BTN_LO);
    } else {
        c.raised(x, y, w, h, BTN_HI, BTN_LO);
    }
}

/// Two gold (active) or grey (inactive) grooves across a title bar.
fn grooves(c: &mut Canvas, x: i32, y: i32, w: i32, active: bool) {
    if w <= 0 {
        return;
    }
    let (hi, lo) = if active { (AMBER_HI, AMBER_LO) } else { (GREY_LINE, GREY_LINE_LO) };
    for (dy, col) in [(0, hi), (1, lo), (3, hi), (4, lo)] {
        c.hline(x, y + dy, w, col);
    }
}

/// A title bar: grooves either side of centred text.
fn title_bar(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, text: &str, active: bool, left_pad: i32, right_pad: i32) {
    c.vgrad(x, y, w, h, mix(BODY, BODY_HI, 0.25), BODY);
    c.hline(x, y, w, BODY_HI);
    c.hline(x, y + h - 1, w, BODY_LO);
    let tw = text_width(text);
    let tx = x + (w - tw) / 2;
    let gy = y + h / 2 - 2;
    grooves(c, x + left_pad, gy, tx - 4 - (x + left_pad), active);
    grooves(c, tx + tw + 4, gy, x + w - right_pad - (tx + tw + 4), active);
    c.text(tx, y + (h - 5) / 2, text, if active { AMBER } else { LABEL });
}

/// A title-bar piece with unbroken grooves from `from` to `to` (relative),
/// so tiles join seamlessly.
fn bar_piece(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, active: bool, from: i32, to: i32) {
    c.vgrad(x, y, w, h, mix(BODY, BODY_HI, 0.25), BODY);
    c.hline(x, y, w, BODY_HI);
    c.hline(x, y + h - 1, w, BODY_LO);
    grooves(c, x + from, y + h / 2 - 2, to - from, active);
}

/// A bottom-edge piece: lines only along the top and bottom, plus the
/// outer side for corners, so tiles join without seams.
fn edge_piece(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, left: bool, right: bool) {
    c.vgrad(x, y, w, h, BODY, BODY_DARK);
    c.hline(x, y, w, BODY_HI);
    c.hline(x, y + h - 1, w, BODY_LO);
    if left {
        c.vline(x, y, h, BODY_HI);
    }
    if right {
        c.vline(x + w - 1, y, h, BODY_LO);
    }
}

/// A 9×9 title-bar button with a symbol.
fn small_button(c: &mut Canvas, x: i32, y: i32, symbol: &[&str], down: bool) {
    c.rect(x, y, 9, 9, if down { BTN_DOWN } else { BTN });
    if down {
        c.sunken(x, y, 9, 9, BTN_HI, BTN_LO);
    } else {
        c.raised(x, y, 9, 9, BTN_HI, BTN_LO);
    }
    c.pattern(x + 2, y + 2, symbol, ICON, ICON);
}

const SYM_CLOSE: &[&str] = &["#...#", ".#.#.", "..#..", ".#.#.", "#...#"];
const SYM_MIN: &[&str] = &[".....", ".....", ".....", ".....", "#####"];
const SYM_SHADE: &[&str] = &["#####", "#...#", "#####", ".....", "....."];
const SYM_UNSHADE: &[&str] = &["#####", "#...#", "#...#", "#...#", "#####"];
const SYM_MENU: &[&str] = &["#####", ".....", "#####", ".....", "#####"];

/// A sunken black display area.
fn lcd(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32) {
    c.rect(x, y, w, h, LCD);
    c.sunken(x - 1, y - 1, w + 2, h + 2, LCD_EDGE_HI, BODY_LO);
}

/// The window body with a bevelled edge.
fn body(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32) {
    c.vgrad(x, y, w, h, BODY, BODY_DARK);
    c.raised(x, y, w, h, BODY_HI, BODY_LO);
}

// ---- seven-segment digits (NUMBERS.BMP) ----------------------------------

/// Segments a..g lit for each digit.
const SEGMENTS: [&str; 10] = ["abcdef", "bc", "abged", "abgcd", "fgbc", "afgcd", "afgedc", "abc", "abcdefg", "abcdfg"];

fn digit(c: &mut Canvas, x: i32, y: i32, d: Option<usize>) {
    c.rect(x, y, 9, 13, LCD);
    let lit = d.map(|d| SEGMENTS[d]).unwrap_or("");
    let seg = |c: &mut Canvas, s: char, on: bool| {
        // Unlit segments only just show, like a real LCD.
        let col = if on { GREEN } else { [10, 26, 12] };
        match s {
            'a' => c.rect(x + 2, y, 5, 2, col),
            'g' => {
                c.rect(x + 2, y + 5, 5, 1, col);
                c.rect(x + 2, y + 6, 5, 1, col);
            }
            'd' => c.rect(x + 2, y + 11, 5, 2, col),
            'f' => c.rect(x + 1, y + 1, 2, 5, col),
            'b' => c.rect(x + 6, y + 1, 2, 5, col),
            'e' => c.rect(x + 1, y + 7, 2, 5, col),
            'c' => c.rect(x + 6, y + 7, 2, 5, col),
            _ => {}
        }
    };
    for s in "abcdefg".chars() {
        seg(c, s, lit.contains(s));
    }
}

fn numbers(ex: bool) -> Canvas {
    let mut c = Canvas::new(if ex { 108 } else { 90 }, 13, LCD);
    for d in 0..10 {
        digit(&mut c, d as i32 * 9, 0, Some(d));
    }
    if ex {
        digit(&mut c, 90, 0, None);
        c.rect(90, 0, 9, 13, LCD);
        c.rect(99, 0, 9, 13, LCD);
        c.rect(101, 6, 5, 1, GREEN);
    }
    c
}

// ---- TEXT.BMP --------------------------------------------------------------

fn text_bmp(font: &serde_json::Map<String, serde_json::Value>) -> Canvas {
    let mut c = Canvas::new(155, 18, LCD);
    for (ch, pos) in font {
        let (row, col) = (pos[0].as_i64().unwrap() as i32, pos[1].as_i64().unwrap() as i32);
        let ch = ch.chars().next().unwrap();
        // Glyphs 4 wide sit at the cell's left; 5-wide fill it.
        c.pattern(col * 5, row * 6, font::glyph(ch), GREEN, GREEN);
    }
    c
}

/// 3×5 capitals for tight labels.
fn tiny(c: &mut Canvas, x: i32, y: i32, s: &str, col: Rgb) {
    let mut cx = x;
    for ch in s.chars() {
        let g: &[&str] = match ch {
            'S' => &["###", "#..", "###", "..#", "###"],
            'T' => &["###", ".#.", ".#.", ".#.", ".#."],
            'E' => &["###", "#..", "##.", "#..", "###"],
            'R' => &["##.", "#.#", "##.", "#.#", "#.#"],
            'O' => &["###", "#.#", "#.#", "#.#", "###"],
            'M' => &["#.#", "###", "#.#", "#.#", "#.#"],
            'N' => &["##.", "#.#", "#.#", "#.#", "#.#"],
            _ => &["...", "...", "...", "...", "..."],
        };
        c.pattern(cx, y, g, col, col);
        cx += 4;
    }
}

// ---- MAIN.BMP --------------------------------------------------------------

fn main_bmp() -> Canvas {
    let mut c = Canvas::new(275, 116, BODY);
    body(&mut c, 0, 0, 275, 116);
    // The display: time and visualizer on the left, song info on the right.
    lcd(&mut c, 22, 22, 81, 39);
    lcd(&mut c, 110, 24, 156, 12);
    lcd(&mut c, 110, 41, 18, 9);
    lcd(&mut c, 155, 41, 13, 9);
    c.text(130, 43, "KBPS", LABEL);
    c.text(170, 43, "KHZ", LABEL);
    // Slider, button and position grooves are drawn by their own sprites;
    // a soft shelf under the transport buttons.
    c.hline(10, 84, 255, BODY_LO);
    c.hline(10, 85, 255, mix(BODY, BODY_HI, 0.4));
    // The logo (about box) in the corner: a small pixel G.
    let g = ["..###.", ".#...#", "#.....", "#..###", "#....#", ".#...#", "..###."];
    c.pattern(255, 94, &g, AMBER, AMBER);
    c
}

// ---- TITLEBAR.BMP ----------------------------------------------------------

fn titlebar_bmp() -> Canvas {
    let mut c = Canvas::new(344, 87, BODY);
    for (y, active) in [(0, true), (15, false), (57, true), (72, false)] {
        title_bar(&mut c, 27, y, 275, 14, "GAMP", active, 18, 34);
        // The options and minimize buttons are part of the bar (only
        // their pressed looks are separate sprites).
        small_button(&mut c, 27 + 6, y + 3, SYM_MENU, false);
        small_button(&mut c, 27 + 244, y + 3, SYM_MIN, false);
    }
    // Shade mode bars (with a mini display).
    for (y, active) in [(29, true), (42, false)] {
        title_bar(&mut c, 27, y, 275, 14, "", active, 18, 34);
        c.rect(27 + 79, y + 3, 38, 8, LCD);
        c.rect(27 + 125, y + 3, 30, 8, LCD);
        for (i, sym) in [&["#....", "#.#..", "###..", "#.#..", "#...."][..], &["#....", "##...", "###..", "##...", "#...."], &["#.#..", "#.#..", "#.#..", "#.#..", "#.#.."], &["###..", "###..", "###..", ".....", "....."], &["#...#", "#..##", "#.###", "#..##", "#...#"], &["..#..", ".###.", "#####", ".....", "#####"]].iter().enumerate() {
            let xs = [169, 176, 186, 195, 204, 215][i];
            c.pattern(27 + xs + 1, y + 4, sym, if active { ICON } else { LABEL }, ICON);
        }
        c.rect(27 + 226, y + 4, 17, 7, LCD);
    }
    small_button(&mut c, 0, 0, SYM_MENU, false);
    small_button(&mut c, 0, 9, SYM_MENU, true);
    small_button(&mut c, 9, 0, SYM_MIN, false);
    small_button(&mut c, 9, 9, SYM_MIN, true);
    small_button(&mut c, 0, 18, SYM_SHADE, false);
    small_button(&mut c, 9, 18, SYM_SHADE, true);
    small_button(&mut c, 18, 0, SYM_CLOSE, false);
    small_button(&mut c, 18, 9, SYM_CLOSE, true);
    small_button(&mut c, 0, 27, SYM_UNSHADE, false);
    small_button(&mut c, 9, 27, SYM_UNSHADE, true);
    // Shade position slider.
    c.rect(0, 36, 17, 7, LCD);
    c.hline(0, 39, 17, GREEN_DIM);
    for x in [17, 20, 23] {
        c.rect(x, 36, 3, 7, AMBER);
    }
    // The clutter bar (O A I D V), normal and disabled, and lit letters.
    for (x, col) in [(304, LABEL), (312, GREY_LINE_LO)] {
        c.rect(x, 0, 8, 43, BODY_DARK);
        c.raised(x, 0, 8, 43, BODY_HI, BODY_LO);
        for (i, l) in ["O", "A", "I", "D", "V"].iter().enumerate() {
            let y = [3, 11, 18, 25, 33][i];
            c.pattern(x + 2, y + 1, &mini(l), col, col);
        }
    }
    for (i, l) in ["O", "A", "I", "D", "V"].iter().enumerate() {
        let (x, y, h) = [(304, 47, 8), (312, 55, 7), (320, 62, 7), (328, 69, 8), (336, 77, 7)][i];
        c.rect(x, y, 8, h, BTN_DOWN);
        c.pattern(x + 2, y + 1, &mini(l), AMBER, AMBER);
    }
    c
}

/// 3×5 letters for the clutter bar.
fn mini(l: &str) -> Vec<&'static str> {
    match l {
        "O" => vec!["###", "#.#", "#.#", "#.#", "###"],
        "A" => vec!["###", "#.#", "###", "#.#", "#.#"],
        "I" => vec!["###", ".#.", ".#.", ".#.", "###"],
        "D" => vec!["##.", "#.#", "#.#", "#.#", "##."],
        _ => vec!["#.#", "#.#", "#.#", "#.#", ".#."],
    }
}

// ---- CBUTTONS.BMP ----------------------------------------------------------

fn cbuttons_bmp() -> Canvas {
    let mut c = Canvas::new(136, 36, BODY);
    let icons: [&[&str]; 6] = [
        &["#....#", "#...##", "#..###", "#.####", "#..###", "#...##", "#....#"],
        &["#....", "##...", "###..", "####.", "###..", "##...", "#...."],
        &["##.##", "##.##", "##.##", "##.##", "##.##", "##.##", "##.##"],
        &["#####", "#####", "#####", "#####", "#####", "#####", "#####"],
        &["#....#", "##...#", "###..#", "####.#", "###..#", "##...#", "#....#"],
        &["...#...", "..###..", ".#####.", "#######", ".......", "#######", "......."],
    ];
    for (i, icon) in icons.iter().enumerate() {
        let (x, w, h) = if i < 5 { (i as i32 * 23, if i == 4 { 22 } else { 23 }, 18) } else { (114, 22, 16) };
        for down in [false, true] {
            let y = if down { h } else { 0 };
            button(&mut c, x, y, w, h, down);
            let iw = icon[0].len() as i32;
            let ih = icon.len() as i32;
            let o = down as i32;
            c.pattern(x + (w - iw) / 2 + o, y + (h - ih) / 2 + o, icon, ICON, ICON);
        }
    }
    c
}

// ---- small state sprites ---------------------------------------------------

fn playpaus_bmp() -> Canvas {
    let mut c = Canvas::new(48, 9, LCD);
    c.pattern(1, 1, &["#....", "##...", "###..", "####.", "###..", "##...", "#...."], GREEN, GREEN);
    c.pattern(10, 1, &["##.##", "##.##", "##.##", "##.##", "##.##", "##.##", "##.##"], AMBER, AMBER);
    c.rect(20, 2, 5, 5, LABEL);
    c.rect(36, 0, 3, 9, LCD);
    c.rect(39, 0, 9, 9, LCD);
    c.rect(40, 2, 1, 5, GREEN);
    c
}

fn monoster_bmp() -> Canvas {
    let mut c = Canvas::new(56, 24, BODY);
    for (y, lit) in [(0, true), (12, false)] {
        let col = if lit { GREEN } else { GREEN_DIM };
        c.rect(0, y, 29, 12, LCD);
        tiny(&mut c, 2, y + 4, "STEREO", col);
        c.rect(29, y, 27, 12, LCD);
        tiny(&mut c, 34, y + 4, "MONO", col);
    }
    c
}

fn posbar_bmp() -> Canvas {
    let mut c = Canvas::new(307, 10, BODY);
    c.rect(0, 0, 248, 10, BODY_DARK);
    c.sunken(0, 2, 248, 6, BODY_HI, BODY_LO);
    c.hline(1, 4, 246, mix(BODY_DARK, BODY_LO, 0.5));
    for (x, sel) in [(248, false), (278, true)] {
        button(&mut c, x, 0, 29, 10, sel);
        c.rect(x + 5, 3, 19, 4, if sel { AMBER_HI } else { AMBER });
        c.hline(x + 5, 3, 19, AMBER_HI);
        c.hline(x + 5, 6, 19, AMBER_LO);
    }
    c
}

/// VOLUME.BMP or BALANCE.BMP: 28 frames whose fill grows with the level.
fn slider_frames(balance: bool) -> Canvas {
    let (w, x0) = if balance { (47, 9) } else { (68, 0) };
    let fw = if balance { 38 } else { 68 };
    let mut c = Canvas::new(w, 433, BODY);
    for i in 0..28 {
        let y = i * 15;
        let t = i as f32 / 27.0;
        c.rect(x0, y, fw, 13, BODY_DARK);
        c.sunken(x0, y + 3, fw, 7, BODY_HI, BODY_LO);
        // Green when low/centred, through amber to red when loud/off-centre.
        let col = if t < 0.5 { mix(GREEN, AMBER, t * 2.0) } else { mix(AMBER, [230, 50, 30], (t - 0.5) * 2.0) };
        if balance {
            let half = fw / 2;
            let n = ((half - 1) as f32 * t) as i32;
            c.rect(x0 + half - n, y + 4, n * 2 + 1, 5, col);
        } else {
            let n = ((fw - 2) as f32 * (0.05 + 0.95 * t)) as i32;
            c.rect(x0 + 1, y + 4, n, 5, col);
        }
    }
    for (x, sel) in [(15, false), (0, true)] {
        button(&mut c, x, 422, 14, 11, sel);
        c.vline(x + 6, 424, 7, BTN_LO);
        c.vline(x + 7, 424, 7, BTN_HI);
    }
    c
}

fn shufrep_bmp() -> Canvas {
    let mut c = Canvas::new(92, 85, BODY);
    for (row, (down, lit)) in [(false, false), (true, false), (false, true), (true, true)].iter().enumerate() {
        let y = row as i32 * 15;
        button(&mut c, 0, y, 28, 15, *down);
        button(&mut c, 28, y, 47, 15, *down);
        let o = *down as i32;
        let led = if *lit { GREEN } else { GREEN_DIM };
        c.rect(4 + o, y + 6 + o, 3, 3, led);
        c.text(9 + o, y + 5 + o, "REP", ICON);
        c.rect(32 + o, y + 6 + o, 3, 3, led);
        c.text(37 + o, y + 5 + o, "SHUFFLE", ICON);
    }
    for (x, label) in [(0, "EQ"), (23, "PL")] {
        for (dx, dy, down, lit) in [(0, 61, false, false), (0, 73, false, true), (46, 61, true, false), (46, 73, true, true)] {
            button(&mut c, x + dx, dy, 23, 12, down);
            let o = down as i32;
            c.rect(x + dx + 4 + o, dy + 5 + o, 3, 3, if lit { GREEN } else { GREEN_DIM });
            c.text(x + dx + 9 + o, dy + 4 + o, label, ICON);
        }
    }
    c
}

// ---- EQMAIN.BMP / EQ_EX.BMP ------------------------------------------------

fn eqmain_bmp() -> Canvas {
    let mut c = Canvas::new(275, 315, BODY);
    body(&mut c, 0, 0, 275, 116);
    // Labels and the band names.
    c.text(48, 39, "+12", LABEL);
    c.text(52, 66, "0", LABEL);
    c.text(48, 94, "-12", LABEL);
    for (i, l) in ["60", "170", "310", "600", "1K", "3K", "6K", "12K", "14K", "16K"].iter().enumerate() {
        let x = 78 + 18 * i as i32 + 7 - text_width(l) / 2;
        c.text(x, 104, l, LABEL);
    }
    c.text(14, 104, "PREAMP", LABEL);
    lcd(&mut c, 86, 17, 113, 19);
    // Title bars.
    for y in [134, 149] {
        title_bar(&mut c, 0, y, 275, 14, "GAMP EQUALIZER", y == 134, 6, 24);
        small_button(&mut c, 254, y + 3, SYM_SHADE, false);
        small_button(&mut c, 264, y + 3, SYM_CLOSE, false);
    }
    small_button(&mut c, 0, 116, SYM_CLOSE, false);
    small_button(&mut c, 0, 125, SYM_CLOSE, true);
    small_button(&mut c, 254, 152, SYM_UNSHADE, true);
    // ON / AUTO buttons: x for (normal, depressed, selected, selected+depressed).
    for (label, w, xs) in [("ON", 26, [10, 128, 69, 187]), ("AUTO", 32, [36, 154, 95, 213])] {
        for (i, x) in xs.iter().enumerate() {
            let down = i % 2 == 1;
            let lit = i >= 2;
            button(&mut c, *x, 119, w, 12, down);
            let o = down as i32;
            c.rect(x + 3 + o, 124 + o, 3, 3, if lit { GREEN } else { GREEN_DIM });
            c.text(x + 8 + o, 123 + o, label, ICON);
        }
    }
    // Slider frames: 28 of 14×63, 14 per row.
    for i in 0..28 {
        let (x, y) = (13 + (i % 14) * 15, 164 + (i / 14) * 65);
        c.rect(x, y, 14, 63, BODY_DARK);
        c.sunken(x + 5, y, 4, 63, BODY_HI, BODY_LO);
        let t = i as f32 / 27.0;
        let col = mix([230, 50, 30], GREEN, t);
        let level = (61.0 * (1.0 - t)) as i32;
        c.rect(x + 6, y + 1 + level, 2, 61 - level, col);
    }
    for (y, sel) in [(164, false), (176, true)] {
        button(&mut c, 0, y, 11, 11, sel);
        c.hline(2, y + 5, 7, if sel { AMBER } else { BTN_LO });
    }
    for (y, down) in [(164, false), (176, true)] {
        button(&mut c, 224, y, 44, 12, down);
        c.text(228 + down as i32, y + 4 + down as i32, "PRESETS", ICON);
    }
    // Response graph background, its line colours (top red to bottom green)
    // and the preamp line.
    c.rect(0, 294, 113, 19, LCD);
    for y in [294, 303, 312] {
        c.hline(0, y, 113, GREEN_DIM);
    }
    for i in 0..19 {
        c.px(115, 294 + i, mix([255, 80, 40], GREEN, i as f32 / 18.0));
    }
    c.hline(0, 314, 113, AMBER_LO);
    c
}

fn eq_ex_bmp() -> Canvas {
    let mut c = Canvas::new(275, 56, BODY);
    for y in [0, 15] {
        title_bar(&mut c, 0, y, 275, 14, "", y == 0, 6, 24);
        small_button(&mut c, 254, y + 3, SYM_UNSHADE, false);
        small_button(&mut c, 264, y + 3, SYM_CLOSE, false);
    }
    for y in [0, 15] {
        c.rect(61, y + 4, 97, 6, LCD);
        c.rect(164, y + 4, 43, 6, LCD);
    }
    for (i, x) in [1, 4, 7, 11, 14, 17].iter().enumerate() {
        c.rect(*x, 30, 3, 7, if i % 3 == 1 { AMBER } else { AMBER_LO });
    }
    small_button(&mut c, 1, 38, SYM_SHADE, true);
    small_button(&mut c, 1, 47, SYM_UNSHADE, true);
    small_button(&mut c, 11, 38, SYM_CLOSE, false);
    small_button(&mut c, 11, 47, SYM_CLOSE, true);
    c
}

// ---- PLEDIT.BMP ------------------------------------------------------------

fn pledit_bmp() -> Canvas {
    let mut c = Canvas::new(280, 186, BODY);
    for (y, active) in [(0, true), (21, false)] {
        // Corners and tiles of the top edge, grooves running through.
        bar_piece(&mut c, 0, y, 25, 20, active, 6, 25);
        bar_piece(&mut c, 127, y, 25, 20, active, 0, 25);
        bar_piece(&mut c, 153, y, 25, 20, active, 0, 3);
        title_bar(&mut c, 26, y, 100, 20, "GAMP PLAYLIST", active, 0, 0);
        // The right corner holds shade and close.
        small_button(&mut c, 153 + 25 - 21, y + 3, SYM_SHADE, false);
        small_button(&mut c, 153 + 25 - 11, y + 3, SYM_CLOSE, false);
        c.vline(0, y, 20, BODY_HI);
        c.vline(177, y, 20, BODY_LO);
    }
    // Left and right edges (the right one holds the scrollbar track).
    c.rect(0, 42, 12, 29, BODY);
    c.vline(0, 42, 29, BODY_HI);
    c.vline(10, 42, 29, BODY_LO);
    c.vline(11, 42, 29, BODY_DARK);
    c.rect(31, 42, 20, 29, BODY);
    c.rect(36, 42, 10, 29, BODY_DARK);
    c.vline(36, 42, 29, BODY_LO);
    c.vline(50, 42, 29, BODY_LO);
    // Scroll handles.
    for (x, sel) in [(52, false), (61, true)] {
        button(&mut c, x, 53, 8, 18, sel);
        c.hline(x + 2, 61, 4, if sel { AMBER } else { BTN_LO });
    }
    // Close / collapse / expand (title bar buttons when selected).
    small_button(&mut c, 52, 42, SYM_CLOSE, true);
    small_button(&mut c, 62, 42, SYM_SHADE, true);
    small_button(&mut c, 150, 42, SYM_UNSHADE, true);
    // Shade mode backgrounds.
    for (x, y, w, active) in [(72, 57, 25, false), (72, 42, 25, true), (99, 57, 50, false), (99, 42, 50, true)] {
        title_bar(&mut c, x, y, w, 14, "", active, 0, 0);
    }
    // Bottom edge: tile, corners, visualizer box.
    edge_piece(&mut c, 179, 0, 25, 38, false, false);
    edge_piece(&mut c, 0, 72, 125, 38, true, false);
    edge_piece(&mut c, 126, 72, 150, 38, false, true);
    body(&mut c, 205, 0, 75, 38);
    lcd(&mut c, 208, 3, 72, 32);
    for (i, label) in ["ADD", "REM", "SEL", "MISC"].iter().enumerate() {
        let x = 14 + 29 * i as i32;
        button(&mut c, x, 72 + 8, 22, 18, false);
        c.text(x + (22 - text_width(label)) / 2, 72 + 8 + 7, label, ICON);
    }
    // Right corner: LIST button, running time, mini transport, mini time.
    button(&mut c, 126 + 106, 72 + 8, 22, 18, false);
    c.text(126 + 106 + 3, 72 + 8 + 4, "LIST", ICON);
    c.text(126 + 106 + 3, 72 + 8 + 10, "OPTS", ICON);
    lcd(&mut c, 126 + 7, 72 + 10, 91, 7);
    for (i, sym) in [&["#....", "#.#..", "###..", "#.#..", "#...."][..], &["#....", "##...", "###..", "##...", "#...."], &["#.#..", "#.#..", "#.#..", "#.#..", "#.#.."], &["###..", "###..", "###..", ".....", "....."], &["#.#..", "#.##.", "#.###", "#.##.", "#.#.."], &["..#..", ".###.", "#####", ".....", "#####"]].iter().enumerate() {
        c.pattern(126 + 3 + 10 * i as i32 + 2, 72 + 22 + 2, sym, ICON, ICON);
    }
    lcd(&mut c, 126 + 66, 72 + 23, 25, 6);
    // Menu items (ADD: URL DIR FILE; REM: ALL CROP SEL MISC; SEL: INV ZERO
    // ALL; MISC: SORT INFO OPTS; LIST: NEW SAVE LOAD), plain and selected.
    let menus: [(i32, &[&str]); 5] = [
        (0, &["URL", "DIR", "FILE"]),
        (54, &["ALL", "CROP", "SEL", "MISC"]),
        (104, &["INV", "NONE", "ALL"]),
        (154, &["SORT", "INFO", "OPTS"]),
        (204, &["NEW", "SAVE", "LOAD"]),
    ];
    for (x, items) in menus {
        for (i, label) in items.iter().enumerate() {
            let y = 111 + 19 * i as i32;
            for (dx, sel) in [(0, false), (23, true)] {
                c.rect(x + dx, y, 22, 18, if sel { BTN_HI } else { BTN });
                c.raised(x + dx, y, 22, 18, BTN_HI, BTN_LO);
                c.text(x + dx + (22 - text_width(label)) / 2, y + 7, label, if sel { BTN_LO } else { ICON });
            }
        }
        // The bar beside each menu.
        let bar_x = x + 46 + if x == 0 { 2 } else { 0 };
        let bar_h = if items.len() == 4 { 72 } else { 54 };
        c.rect(bar_x, 111, 3, bar_h, AMBER_LO);
        c.vline(bar_x, 111, bar_h, AMBER);
    }
    c
}

// ---- GEN.BMP -----------------------------------------------------------------

fn gen_bmp() -> Canvas {
    let mut c = Canvas::new(210, 104, BODY);
    for (y, active) in [(0, true), (21, false)] {
        // Left corner, left end (beside the title), centre fill (behind it),
        // right end, left/right fills, right corner (with close).
        bar_piece(&mut c, 0, y, 25, 20, active, 6, 25);
        bar_piece(&mut c, 26, y, 25, 20, active, 0, 21);
        c.vgrad(52, y, 25, 20, mix(BODY, BODY_HI, 0.25), BODY);
        c.hline(52, y, 25, BODY_HI);
        c.hline(52, y + 19, 25, BODY_LO);
        bar_piece(&mut c, 78, y, 25, 20, active, 4, 25);
        bar_piece(&mut c, 104, y, 25, 20, active, 0, 25);
        bar_piece(&mut c, 130, y, 25, 20, active, 0, 12);
        small_button(&mut c, 130 + 14, y + 3, SYM_CLOSE, false);
        c.vline(0, y, 20, BODY_HI);
        c.vline(154, y, 20, BODY_LO);
    }
    edge_piece(&mut c, 0, 42, 125, 14, true, false);
    edge_piece(&mut c, 0, 57, 125, 14, false, true);
    edge_piece(&mut c, 127, 72, 25, 14, false, false);
    for (x, w, y, h) in [(127, 11, 42, 29), (158, 11, 42, 24), (139, 8, 42, 29), (170, 8, 42, 24)] {
        c.rect(x, y, w, h, BODY);
        let left = x == 127 || x == 158;
        c.vline(if left { x } else { x + w - 1 }, y, h, if left { BODY_HI } else { BODY_LO });
    }
    small_button(&mut c, 148, 42, SYM_CLOSE, true);
    // Title letters A–Z: active row 88, inactive row 96. Each letter sits on
    // its own backing colour; columns of the sheet background separate them.
    for (y, col) in [(88, AMBER), (96, LABEL)] {
        let mut x = 1;
        for l in 'A'..='Z' {
            let w = font::width(l) as i32;
            c.rect(x, y, w, 7, BODY_DARK);
            c.pattern(x, y + 1, font::glyph(l), col, col);
            x += w + 1;
        }
    }
    c
}

// ---- text files --------------------------------------------------------------

const PLEDIT_TXT: &str = "[Text]\r\nNormal=#2CEC3C\r\nCurrent=#FFE278\r\nNormalBG=#000000\r\nSelectedBG=#2E2E6A\r\nFont=Arial\r\n";

fn viscolor_txt() -> String {
    let mut lines = vec!["0,0,0, // background".to_owned(), "28,28,44, // dots".to_owned()];
    for i in 0..16 {
        let t = i as f32 / 15.0;
        let c = if t < 0.5 { mix([255, 70, 40], AMBER, t * 2.0) } else { mix(AMBER, GREEN, (t - 0.5) * 2.0) };
        lines.push(format!("{},{},{}, // analyzer {}", c[0], c[1], c[2], i + 1));
    }
    for i in 0..5 {
        let c = mix(AMBER_HI, AMBER_LO, i as f32 / 4.0);
        lines.push(format!("{},{},{}, // oscilloscope {}", c[0], c[1], c[2], i + 1));
    }
    lines.push("255,255,255, // peak dots".into());
    lines.join("\r\n") + "\r\n"
}

const README: &str = "GAMP Classic\r\nThe default skin of Gamp (https://github.com/fillinthe/g-amp).\r\nDrawn from code by tools/skin-gen. MIT licence.\r\n";

// ---- output ----------------------------------------------------------------

fn encode_bmp(c: &Canvas) -> Vec<u8> {
    let rgb = image::DynamicImage::ImageRgba8(c.img.clone()).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    rgb.write_to(&mut out, image::ImageFormat::Bmp).expect("bmp");
    out.into_inner()
}

fn main() {
    let table: serde_json::Value = serde_json::from_str(include_str!("../../../crates/gamp-skin/src/sprites.json")).expect("sprites.json");
    let sheets: Vec<(&str, Canvas)> = vec![
        ("main.bmp", main_bmp()),
        ("titlebar.bmp", titlebar_bmp()),
        ("cbuttons.bmp", cbuttons_bmp()),
        ("text.bmp", text_bmp(table["font"].as_object().unwrap())),
        ("numbers.bmp", numbers(false)),
        ("nums_ex.bmp", numbers(true)),
        ("playpaus.bmp", playpaus_bmp()),
        ("monoster.bmp", monoster_bmp()),
        ("posbar.bmp", posbar_bmp()),
        ("volume.bmp", slider_frames(false)),
        ("balance.bmp", slider_frames(true)),
        ("shufrep.bmp", shufrep_bmp()),
        ("eqmain.bmp", eqmain_bmp()),
        ("eq_ex.bmp", eq_ex_bmp()),
        ("pledit.bmp", pledit_bmp()),
        ("gen.bmp", gen_bmp()),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let png_dir = std::env::args().nth(1);
    let out = root.join("assets/skins/gamp-classic.wsz");
    let file = std::fs::File::create(&out).expect("create the skin file");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, canvas) in &sheets {
        zip.start_file(*name, opts).unwrap();
        zip.write_all(&encode_bmp(canvas)).unwrap();
        if let Some(dir) = &png_dir {
            std::fs::create_dir_all(dir).unwrap();
            canvas.img.save(std::path::Path::new(dir).join(name.replace(".bmp", ".png"))).unwrap();
        }
    }
    for (name, text) in [("pledit.txt", PLEDIT_TXT.to_owned()), ("viscolor.txt", viscolor_txt()), ("readme.txt", README.to_owned())] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    println!("wrote {}", out.display());
}
