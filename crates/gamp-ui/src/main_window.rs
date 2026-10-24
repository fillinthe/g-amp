//! The main window: transport, time, title, sliders, visualizer. Positions
//! are Winamp 2's (see docs/winamp-layout.md).

use gamp_skin::Skin;

use crate::frame::Frame;
use crate::vis::{self, Analyzer, VisMode};

pub const WIDTH: u32 = 275;
pub const HEIGHT: u32 = 116;
pub const SHADE_HEIGHT: u32 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// Everything you can press or drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    TitleBar,
    Options,
    Minimize,
    Shade,
    Close,
    Clutter(char),
    Time,
    Vis,
    Marquee,
    Volume,
    Balance,
    Equalizer,
    Playlist,
    Position,
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Shuffle,
    Repeat,
    About,
}

/// What the window shows.
#[derive(Clone, Default)]
pub struct View {
    pub state: PlayState,
    pub elapsed: f64,
    pub duration: Option<f64>,
    pub show_remaining: bool,
    /// Marquee text, already formatted ("1. Artist - Title (3:25)").
    pub title: String,
    /// Pixels scrolled.
    pub scroll: i32,
    /// A message replacing the title while a slider is dragged.
    pub message: Option<String>,
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    pub stereo: Option<bool>,
    /// 0..1
    pub volume: f32,
    /// -1..1
    pub balance: f32,
    /// While dragging the seek bar: where it is held (0..1).
    pub seeking: Option<f32>,
    pub shuffle: bool,
    pub repeat: bool,
    pub eq_open: bool,
    pub playlist_open: bool,
    pub focused: bool,
    pub shade: bool,
    pub pressed: Option<Control>,
    pub vis_mode: VisMode,
    pub analyzer: Analyzer,
    pub wave: Vec<f32>,
    /// Blink phase while paused (time shown when true).
    pub blink_on: bool,
}

pub fn hit(x: i32, y: i32, shade: bool) -> Option<Control> {
    let inside = |l: i32, t: i32, w: i32, h: i32| x >= l && y >= t && x < l + w && y < t + h;
    if inside(6, 3, 9, 9) {
        return Some(Control::Options);
    }
    if inside(244, 3, 9, 9) {
        return Some(Control::Minimize);
    }
    if inside(254, 3, 9, 9) {
        return Some(Control::Shade);
    }
    if inside(264, 3, 9, 9) {
        return Some(Control::Close);
    }
    if shade {
        let shade_buttons = [
            (169, 7, Control::Previous),
            (176, 10, Control::Play),
            (186, 9, Control::Pause),
            (195, 9, Control::Stop),
            (204, 10, Control::Next),
            (215, 10, Control::Eject),
        ];
        for (l, w, c) in shade_buttons {
            if inside(l, 2, w, 10) {
                return Some(c);
            }
        }
        if inside(226, 4, 17, 7) {
            return Some(Control::Position);
        }
        if inside(79, 5, 38, 5) {
            return Some(Control::Vis);
        }
        return (y < 14).then_some(Control::TitleBar);
    }
    if y < 14 {
        return Some(Control::TitleBar);
    }
    if inside(10, 22, 8, 43) {
        let letter = match y - 22 {
            3..=10 => 'o',
            11..=17 => 'a',
            18..=24 => 'i',
            25..=32 => 'd',
            33..=39 => 'v',
            _ => return None,
        };
        return Some(Control::Clutter(letter));
    }
    let items = [
        (39, 26, 59, 13, Control::Time),
        (24, 43, 76, 16, Control::Vis),
        (111, 24, 154, 12, Control::Marquee),
        (107, 57, 68, 13, Control::Volume),
        (177, 57, 38, 13, Control::Balance),
        (219, 58, 23, 12, Control::Equalizer),
        (242, 58, 23, 12, Control::Playlist),
        (16, 72, 248, 10, Control::Position),
        (16, 88, 23, 18, Control::Previous),
        (39, 88, 23, 18, Control::Play),
        (62, 88, 23, 18, Control::Pause),
        (85, 88, 23, 18, Control::Stop),
        (108, 88, 22, 18, Control::Next),
        (136, 89, 22, 16, Control::Eject),
        (164, 89, 47, 15, Control::Shuffle),
        (210, 89, 28, 15, Control::Repeat),
        (253, 91, 13, 15, Control::About),
    ];
    items.iter().find(|(l, t, w, h, _)| inside(*l, *t, *w, *h)).map(|i| i.4)
}

/// Slider values from a pointer x inside the window.
pub fn volume_at(x: i32) -> f32 {
    ((x - 107 - 7) as f32 / 51.0).clamp(0.0, 1.0)
}

pub fn balance_at(x: i32) -> f32 {
    let v = ((x - 177 - 7) as f32 / 24.0).clamp(0.0, 1.0) * 2.0 - 1.0;
    // Snap to centre, as Winamp does.
    if v.abs() < 0.08 { 0.0 } else { v }
}

pub fn position_at(x: i32, shade: bool) -> f32 {
    if shade { ((x - 226) as f32 / 17.0).clamp(0.0, 1.0) } else { ((x - 16 - 14) as f32 / 219.0).clamp(0.0, 1.0) }
}

/// The time shown: where it plays, or where the position bar is being
/// dragged to. (remaining, hours, minutes, seconds); hours only from an
/// hour on (a long mix), and then minutes are 0-59.
fn time_parts(v: &View) -> (bool, Option<u32>, u32, u32) {
    let at = match (v.seeking, v.duration) {
        (Some(p), Some(d)) => d * p as f64,
        _ => v.elapsed,
    };
    let secs = match (v.show_remaining, v.duration) {
        (true, Some(d)) => (d - at).max(0.0),
        _ => at.max(0.0),
    } as u32;
    let hours = (secs >= 3600).then(|| (secs / 3600).min(9));
    let m = if hours.is_some() { secs / 60 % 60 } else { secs / 60 };
    (v.show_remaining && v.duration.is_some(), hours, m, secs % 60)
}

/// The time as text: "-1:23:45", " 3:07".
fn time_text(v: &View) -> String {
    let (minus, h, m, s) = time_parts(v);
    let sign = if minus { "-" } else { " " };
    match h {
        Some(h) => format!("{sign}{h}:{m:02}:{s:02}"),
        None => format!("{sign}{m:02}:{s:02}"),
    }
}

/// The lit colour of the skin's digits (their brightest pixel).
fn digit_color(skin: &Skin, name: &str) -> Option<u32> {
    let (img, r) = skin.sprite(name)?;
    let mut best = (0u32, 0u32);
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            let p = img.get_pixel(x, y).0;
            let lum = p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32;
            if lum > best.0 {
                best = (lum, 0xFF00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32);
            }
        }
    }
    (best.0 > 0).then_some(best.1)
}

pub fn draw(frame: &mut Frame, skin: &Skin, v: &View) {
    if v.shade {
        return draw_shade(frame, skin, v);
    }
    frame.resize(WIDTH, HEIGHT);
    frame.sprite(skin, "MAIN_WINDOW_BACKGROUND", 0, 0);
    frame.sprite(skin, if v.focused { "MAIN_TITLE_BAR_SELECTED" } else { "MAIN_TITLE_BAR" }, 0, 0);
    let pressed = |c: Control| v.pressed == Some(c);
    if pressed(Control::Options) {
        frame.sprite(skin, "MAIN_OPTIONS_BUTTON_DEPRESSED", 6, 3);
    }
    if pressed(Control::Minimize) {
        frame.sprite(skin, "MAIN_MINIMIZE_BUTTON_DEPRESSED", 244, 3);
    }
    frame.sprite(skin, if pressed(Control::Shade) { "MAIN_SHADE_BUTTON_DEPRESSED" } else { "MAIN_SHADE_BUTTON" }, 254, 3);
    frame.sprite(skin, if pressed(Control::Close) { "MAIN_CLOSE_BUTTON_DEPRESSED" } else { "MAIN_CLOSE_BUTTON" }, 264, 3);
    frame.sprite(skin, "MAIN_CLUTTER_BAR_BACKGROUND", 10, 22);
    if let Some(Control::Clutter(l)) = v.pressed {
        let (name, y) = match l {
            'o' => ("MAIN_CLUTTER_BAR_BUTTON_O_SELECTED", 25),
            'a' => ("MAIN_CLUTTER_BAR_BUTTON_A_SELECTED", 33),
            'i' => ("MAIN_CLUTTER_BAR_BUTTON_I_SELECTED", 40),
            'd' => ("MAIN_CLUTTER_BAR_BUTTON_D_SELECTED", 47),
            _ => ("MAIN_CLUTTER_BAR_BUTTON_V_SELECTED", 55),
        };
        frame.sprite(skin, name, 10, y);
    }

    // State indicator.
    let indicator = match v.state {
        PlayState::Playing => "MAIN_PLAYING_INDICATOR",
        PlayState::Paused => "MAIN_PAUSED_INDICATOR",
        PlayState::Stopped => "MAIN_STOPPED_INDICATOR",
    };
    frame.sprite(skin, indicator, 26, 28);
    if v.state == PlayState::Playing {
        // Only its first 3 columns: the play symbol sits beside it.
        frame.sprite_part(skin, "MAIN_WORKING_INDICATOR", 0, 0, 3, 9, 24, 28);
    }

    // Time.
    if v.state != PlayState::Stopped && (v.state != PlayState::Paused || v.blink_on) {
        let ex = skin.has("NUMS_EX");
        let digit = |d: u32| if ex { format!("DIGIT_{d}_EX") } else { format!("DIGIT_{d}") };
        let (minus, hours, m, s) = time_parts(v);
        if let Some(h) = hours {
            // An hour or more: the hour goes where the minus sign sits, with
            // a colon in the digits' colour; a minus, if any, just before.
            frame.sprite(skin, &digit(h), 35, 26);
            if let Some(c) = digit_color(skin, &digit(8)) {
                frame.fill(45, 29, 2, 2, c);
                frame.fill(45, 34, 2, 2, c);
                // Between minutes and seconds too, unless the skin has one.
                let lit = (26..39).any(|y| (69..78).any(|x| frame.pixels[(y * WIDTH as i32 + x) as usize] == c));
                if !lit {
                    frame.fill(72, 29, 2, 2, c);
                    frame.fill(72, 34, 2, 2, c);
                }
                if minus {
                    frame.fill(33, 32, 2, 1, c);
                }
            }
        } else if minus {
            if ex {
                frame.sprite(skin, "MINUS_SIGN_EX", 39, 26);
            } else {
                frame.sprite(skin, "MINUS_SIGN", 39 - 1 + 0, 26 + 6);
            }
        }
        frame.sprite(skin, &digit(m / 10), 39 + 9, 26);
        frame.sprite(skin, &digit(m % 10), 39 + 21, 26);
        frame.sprite(skin, &digit(s / 10), 39 + 39, 26);
        frame.sprite(skin, &digit(s % 10), 39 + 51, 26);
    }

    // Visualizer.
    vis::draw(frame, &skin.viscolors, 24, 43, v.vis_mode, &v.analyzer, &v.wave);

    // Title (or a slider's message).
    let text = v.message.as_deref().unwrap_or(&v.title).to_uppercase();
    // Too long to fit (a title, or a long message): it scrolls.
    let (padded, scroll) = if text.chars().count() * 5 > 154 { (format!("{text}  ***  "), v.scroll) } else { (text, 0) };
    frame.text(skin, &padded, 111, 27, 154, scroll);

    if v.state != PlayState::Stopped {
        if let Some(k) = v.kbps {
            let k = format!("{:>3}", k.min(999));
            frame.text(skin, &k, 111, 43, 15, 0);
        }
        if let Some(k) = v.khz {
            let k = format!("{:>2}", k.min(99));
            frame.text(skin, &k, 156, 43, 10, 0);
        }
    }
    let stereo = v.stereo.filter(|_| v.state != PlayState::Stopped);
    frame.sprite(skin, if stereo == Some(false) { "MAIN_MONO_SELECTED" } else { "MAIN_MONO" }, 212, 41);
    frame.sprite(skin, if stereo == Some(true) { "MAIN_STEREO_SELECTED" } else { "MAIN_STEREO" }, 239, 41);

    // Volume and balance: the background frame follows the level.
    let vol_frame = ((v.volume * 27.0).round() as u32).min(27);
    frame.sprite_part(skin, "MAIN_VOLUME_BACKGROUND", 0, vol_frame * 15, 68, 13, 107, 57);
    let vol_x = 107 + (v.volume * 51.0).round() as i32;
    frame.sprite(skin, if pressed(Control::Volume) { "MAIN_VOLUME_THUMB_SELECTED" } else { "MAIN_VOLUME_THUMB" }, vol_x, 58);
    let bal_frame = ((v.balance.abs() * 27.0).round() as u32).min(27);
    frame.sprite_part(skin, "MAIN_BALANCE_BACKGROUND", 0, bal_frame * 15, 38, 13, 177, 57);
    let bal_x = 177 + ((v.balance + 1.0) / 2.0 * 24.0).round() as i32;
    frame.sprite(skin, if pressed(Control::Balance) { "MAIN_BALANCE_THUMB_ACTIVE" } else { "MAIN_BALANCE_THUMB" }, bal_x, 58);

    // EQ / PL toggles.
    let eq = match (v.eq_open, pressed(Control::Equalizer)) {
        (false, false) => "MAIN_EQ_BUTTON",
        (true, false) => "MAIN_EQ_BUTTON_SELECTED",
        (false, true) => "MAIN_EQ_BUTTON_DEPRESSED",
        (true, true) => "MAIN_EQ_BUTTON_DEPRESSED_SELECTED",
    };
    frame.sprite(skin, eq, 219, 58);
    let pl = match (v.playlist_open, pressed(Control::Playlist)) {
        (false, false) => "MAIN_PLAYLIST_BUTTON",
        (true, false) => "MAIN_PLAYLIST_BUTTON_SELECTED",
        (false, true) => "MAIN_PLAYLIST_BUTTON_DEPRESSED",
        (true, true) => "MAIN_PLAYLIST_BUTTON_DEPRESSED_SELECTED",
    };
    frame.sprite(skin, pl, 242, 58);

    // Position bar: the thumb shows only while something is loaded.
    frame.sprite(skin, "MAIN_POSITION_SLIDER_BACKGROUND", 16, 72);
    if let (Some(d), true) = (v.duration, v.state != PlayState::Stopped) {
        let p = v.seeking.unwrap_or(if d > 0.0 { (v.elapsed / d) as f32 } else { 0.0 }).clamp(0.0, 1.0);
        let x = 16 + (p * 219.0).round() as i32;
        frame.sprite(skin, if pressed(Control::Position) { "MAIN_POSITION_SLIDER_THUMB_SELECTED" } else { "MAIN_POSITION_SLIDER_THUMB" }, x, 72);
    }

    // Transport.
    for (c, name, x, y) in [
        (Control::Previous, "MAIN_PREVIOUS_BUTTON", 16, 88),
        (Control::Play, "MAIN_PLAY_BUTTON", 39, 88),
        (Control::Pause, "MAIN_PAUSE_BUTTON", 62, 88),
        (Control::Stop, "MAIN_STOP_BUTTON", 85, 88),
        (Control::Next, "MAIN_NEXT_BUTTON", 108, 88),
        (Control::Eject, "MAIN_EJECT_BUTTON", 136, 89),
    ] {
        let n = if pressed(c) { format!("{name}_ACTIVE") } else { name.to_owned() };
        frame.sprite(skin, &n, x, y);
    }
    let toggle = |base: &str, on: bool, down: bool| match (on, down) {
        (false, false) => base.to_owned(),
        (false, true) => format!("{base}_DEPRESSED"),
        (true, false) => format!("{base}_SELECTED"),
        (true, true) => format!("{base}_SELECTED_DEPRESSED"),
    };
    frame.sprite(skin, &toggle("MAIN_SHUFFLE_BUTTON", v.shuffle, pressed(Control::Shuffle)), 164, 89);
    frame.sprite(skin, &toggle("MAIN_REPEAT_BUTTON", v.repeat, pressed(Control::Repeat)), 210, 89);
}

fn draw_shade(frame: &mut Frame, skin: &Skin, v: &View) {
    frame.resize(WIDTH, SHADE_HEIGHT);
    frame.sprite(skin, if v.focused { "MAIN_SHADE_BACKGROUND_SELECTED" } else { "MAIN_SHADE_BACKGROUND" }, 0, 0);
    frame.sprite(skin, if v.pressed == Some(Control::Shade) { "MAIN_SHADE_BUTTON_SELECTED_DEPRESSED" } else { "MAIN_SHADE_BUTTON_SELECTED" }, 254, 3);
    frame.sprite(skin, if v.pressed == Some(Control::Close) { "MAIN_CLOSE_BUTTON_DEPRESSED" } else { "MAIN_CLOSE_BUTTON" }, 264, 3);
    if v.pressed == Some(Control::Minimize) {
        frame.sprite(skin, "MAIN_MINIMIZE_BUTTON_DEPRESSED", 244, 3);
    }
    if v.state != PlayState::Stopped && (v.state != PlayState::Paused || v.blink_on) {
        let t = time_text(v);
        frame.text(skin, &t, 127, 4, t.chars().count() as i32 * 5, 0);
    }
    if let (Some(d), true) = (v.duration, v.state != PlayState::Stopped) {
        let p = v.seeking.unwrap_or(if d > 0.0 { (v.elapsed / d) as f32 } else { 0.0 }).clamp(0.0, 1.0);
        let name = if p < 0.33 { "MAIN_SHADE_POSITION_THUMB_LEFT" } else if p > 0.66 { "MAIN_SHADE_POSITION_THUMB_RIGHT" } else { "MAIN_SHADE_POSITION_THUMB" };
        frame.sprite(skin, name, 226 + (p * 14.0).round() as i32, 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_follows_a_drag_and_reads_long_mixes() {
        let mut v = View { state: PlayState::Playing, elapsed: 83.0, duration: Some(12470.0), ..Default::default() };
        assert_eq!(time_parts(&v), (false, None, 1, 23));
        assert_eq!(time_text(&v), " 01:23");
        // Dragged to 40%: 4988 s, 1:23:08.
        v.seeking = Some(0.4);
        assert_eq!(time_parts(&v), (false, Some(1), 23, 8));
        assert_eq!(time_text(&v), " 1:23:08");
        v.seeking = Some(0.1);
        assert_eq!(time_parts(&v), (false, None, 20, 47));
        v.seeking = None;
        v.elapsed = 59.0 * 60.0 + 59.0;
        assert_eq!(time_parts(&v), (false, None, 59, 59));
        v.elapsed = 3600.0;
        assert_eq!(time_parts(&v), (false, Some(1), 0, 0));
        // Remaining: 12470 - 3600 s left, 2:27:50.
        v.show_remaining = true;
        assert_eq!(time_text(&v), "-2:27:50");
    }

    #[test]
    fn an_hour_or_more_draws_a_fifth_digit_and_a_colon() {
        let skin = Skin::default_skin();
        let draw_at = |elapsed: f64| {
            let mut f = Frame::new(1, 1);
            draw(&mut f, &skin, &View { state: PlayState::Playing, elapsed, duration: Some(12470.0), ..Default::default() });
            f
        };
        let (short, long) = (draw_at(83.0), draw_at(5000.0));
        let px = |f: &Frame, x: i32, y: i32| f.pixels[(y * WIDTH as i32 + x) as usize];
        assert_eq!(px(&long, 45, 29), digit_color(&skin, "DIGIT_8").unwrap());
        assert_ne!(px(&short, 45, 29), px(&long, 45, 29));
        if let Ok(dir) = std::env::var("GAMP_SNAPSHOTS") {
            long.to_image().save(format!("{dir}/main-hours.png")).unwrap();
            // GAMP_SNAPSHOT_SKINS=<folder of .wsz>: the same in each.
            for e in std::env::var("GAMP_SNAPSHOT_SKINS").into_iter().flat_map(|d| std::fs::read_dir(d).unwrap()).flatten() {
                let Ok(other) = Skin::from_wsz(&std::fs::read(e.path()).unwrap(), &skin) else { continue };
                let mut f = Frame::new(1, 1);
                draw(&mut f, &other, &View { state: PlayState::Playing, elapsed: 5000.0, duration: Some(12470.0), ..Default::default() });
                f.to_image().save(format!("{dir}/hours-{}.png", e.file_name().to_string_lossy())).unwrap();
            }
        }
    }

    #[test]
    fn controls_are_where_winamp_put_them() {
        assert_eq!(hit(40, 90, false), Some(Control::Play));
        assert_eq!(hit(150, 5, false), Some(Control::TitleBar));
        assert_eq!(hit(266, 5, false), Some(Control::Close));
        assert_eq!(hit(120, 60, false), Some(Control::Volume));
        assert_eq!(hit(12, 37, false), Some(Control::Clutter('a')));
        assert_eq!(hit(178, 5, true), Some(Control::Play));
        assert_eq!(volume_at(107 + 7), 0.0);
        assert_eq!(volume_at(107 + 7 + 51), 1.0);
        assert_eq!(balance_at(177 + 7 + 12), 0.0);
    }

    #[test]
    fn draws_a_playing_window() {
        let skin = Skin::default_skin();
        let mut f = Frame::new(1, 1);
        let mut v = View { state: PlayState::Playing, elapsed: 83.0, duration: Some(200.0), title: "1. GAMP - TEST (3:20)".into(), kbps: Some(192), khz: Some(44), stereo: Some(true), volume: 0.7, focused: true, ..Default::default() };
        v.analyzer.update(&[0.9; 19]);
        draw(&mut f, &skin, &v);
        assert_eq!((f.width, f.height), (WIDTH, HEIGHT));
        if let Ok(dir) = std::env::var("GAMP_SNAPSHOTS") {
            f.to_image().save(format!("{dir}/main.png")).unwrap();
        }
        // The analyzer is lit somewhere in its box.
        let lit = (43..59).any(|y| (24..100).any(|x| f.pixels[(y * WIDTH as i32 + x) as usize] != f.pixels[(43 * WIDTH as i32 + 24) as usize]));
        assert!(lit);
    }
}
