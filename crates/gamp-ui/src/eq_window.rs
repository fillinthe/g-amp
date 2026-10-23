//! The equalizer window.

use gamp_skin::Skin;

use crate::frame::{Frame, argb};

pub const WIDTH: u32 = 275;
pub const HEIGHT: u32 = 116;
pub const SHADE_HEIGHT: u32 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    TitleBar,
    Shade,
    Close,
    On,
    Auto,
    Presets,
    Preamp,
    Band(usize),
    /// Shade mode sliders.
    Volume,
    Balance,
}

#[derive(Clone, Default)]
pub struct View {
    pub on: bool,
    pub auto: bool,
    /// dB, -12..12
    pub preamp: f32,
    pub bands: [f32; 10],
    pub focused: bool,
    pub shade: bool,
    pub pressed: Option<Control>,
    pub volume: f32,
    pub balance: f32,
}

const SLIDER_Y: i32 = 38;
const TRAVEL: f32 = 51.0;

fn band_x(i: usize) -> i32 {
    78 + 18 * i as i32
}

pub fn hit(x: i32, y: i32, shade: bool) -> Option<Control> {
    let inside = |l: i32, t: i32, w: i32, h: i32| x >= l && y >= t && x < l + w && y < t + h;
    if inside(254, 3, 9, 9) {
        return Some(Control::Shade);
    }
    if inside(264, 3, 9, 9) {
        return Some(Control::Close);
    }
    if shade {
        if inside(61, 3, 97, 8) {
            return Some(Control::Volume);
        }
        if inside(164, 3, 43, 8) {
            return Some(Control::Balance);
        }
        return (y < 14).then_some(Control::TitleBar);
    }
    if y < 14 {
        return Some(Control::TitleBar);
    }
    if inside(14, 18, 26, 12) {
        return Some(Control::On);
    }
    if inside(40, 18, 32, 12) {
        return Some(Control::Auto);
    }
    if inside(217, 18, 44, 12) {
        return Some(Control::Presets);
    }
    if inside(21, SLIDER_Y, 14, 63) {
        return Some(Control::Preamp);
    }
    (0..10).find(|i| inside(band_x(*i), SLIDER_Y, 14, 63)).map(Control::Band)
}

/// dB for a pointer y on a slider; snaps to 0 near the middle.
pub fn db_at(y: i32) -> f32 {
    let norm = 1.0 - ((y - SLIDER_Y - 5) as f32 / TRAVEL).clamp(0.0, 1.0);
    let db = norm * 24.0 - 12.0;
    if db.abs() < 0.7 { 0.0 } else { db }
}

fn slider(frame: &mut Frame, skin: &Skin, x: i32, db: f32, pressed: bool) {
    let norm = ((db + 12.0) / 24.0).clamp(0.0, 1.0);
    let i = (norm * 27.0).round() as u32;
    frame.sprite_part(skin, "EQ_SLIDER_BACKGROUND", (i % 14) * 15, (i / 14) * 65, 14, 63, x, SLIDER_Y);
    let ty = SLIDER_Y + ((1.0 - norm) * TRAVEL).round() as i32;
    frame.sprite(skin, if pressed { "EQ_SLIDER_THUMB_SELECTED" } else { "EQ_SLIDER_THUMB" }, x + 1, ty);
}

/// Natural cubic spline through the band values, sampled per pixel.
fn curve(values: &[f32; 10], width: usize) -> Vec<f32> {
    let n = values.len();
    let xs: Vec<f32> = (0..n).map(|i| i as f32 * (width - 1) as f32 / (n - 1) as f32).collect();
    let h: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
    let mut alpha = vec![0.0; n];
    for i in 1..n - 1 {
        alpha[i] = 3.0 / h[i] * (values[i + 1] - values[i]) - 3.0 / h[i - 1] * (values[i] - values[i - 1]);
    }
    let (mut l, mut mu, mut z) = (vec![1.0; n], vec![0.0; n], vec![0.0; n]);
    for i in 1..n - 1 {
        l[i] = 2.0 * (xs[i + 1] - xs[i - 1]) - h[i - 1] * mu[i - 1];
        mu[i] = h[i] / l[i];
        z[i] = (alpha[i] - h[i - 1] * z[i - 1]) / l[i];
    }
    let (mut c, mut b, mut d) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for j in (0..n - 1).rev() {
        c[j] = z[j] - mu[j] * c[j + 1];
        b[j] = (values[j + 1] - values[j]) / h[j] - h[j] * (c[j + 1] + 2.0 * c[j]) / 3.0;
        d[j] = (c[j + 1] - c[j]) / (3.0 * h[j]);
    }
    (0..width)
        .map(|x| {
            let x = x as f32;
            let j = xs.iter().rposition(|xi| *xi <= x).unwrap_or(0).min(n - 2);
            let dx = x - xs[j];
            values[j] + b[j] * dx + c[j] * dx * dx + d[j] * dx * dx * dx
        })
        .collect()
}

pub fn draw(frame: &mut Frame, skin: &Skin, v: &View) {
    let pressed = |c: Control| v.pressed == Some(c);
    if v.shade {
        frame.resize(WIDTH, SHADE_HEIGHT);
        frame.sprite(skin, if v.focused { "EQ_SHADE_BACKGROUND_SELECTED" } else { "EQ_SHADE_BACKGROUND" }, 0, 0);
        let vx = 61 + (v.volume * 94.0).round() as i32;
        let vname = if v.volume < 0.33 { "EQ_SHADE_VOLUME_SLIDER_LEFT" } else if v.volume > 0.66 { "EQ_SHADE_VOLUME_SLIDER_RIGHT" } else { "EQ_SHADE_VOLUME_SLIDER_CENTER" };
        frame.sprite(skin, vname, vx, 4);
        let bx = 164 + ((v.balance + 1.0) / 2.0 * 40.0).round() as i32;
        let bname = if v.balance < -0.33 { "EQ_SHADE_BALANCE_SLIDER_LEFT" } else if v.balance > 0.33 { "EQ_SHADE_BALANCE_SLIDER_RIGHT" } else { "EQ_SHADE_BALANCE_SLIDER_CENTER" };
        frame.sprite(skin, bname, bx, 4);
        frame.sprite(skin, "EQ_MINIMIZE_BUTTON_ACTIVE", 254, 3);
        frame.sprite(skin, if pressed(Control::Close) { "EQ_SHADE_CLOSE_BUTTON_ACTIVE" } else { "EQ_SHADE_CLOSE_BUTTON" }, 264, 3);
        return;
    }
    frame.resize(WIDTH, HEIGHT);
    frame.sprite(skin, "EQ_WINDOW_BACKGROUND", 0, 0);
    frame.sprite(skin, if v.focused { "EQ_TITLE_BAR_SELECTED" } else { "EQ_TITLE_BAR" }, 0, 0);
    if pressed(Control::Shade) {
        frame.sprite(skin, "EQ_MAXIMIZE_BUTTON_ACTIVE_FALLBACK", 254, 3);
    } else if !v.focused {
        // The unfocused title bar's shade button is the pressed one (the
        // two share pixels in eqmain.bmp); Winamp shows it up.
        frame.sprite(skin, "EQ_SHADE_BUTTON", 254, 3);
    }
    if pressed(Control::Close) {
        frame.sprite(skin, "EQ_CLOSE_BUTTON_ACTIVE", 264, 3);
    }
    let on = match (v.on, pressed(Control::On)) {
        (false, false) => "EQ_ON_BUTTON",
        (false, true) => "EQ_ON_BUTTON_DEPRESSED",
        (true, false) => "EQ_ON_BUTTON_SELECTED",
        (true, true) => "EQ_ON_BUTTON_SELECTED_DEPRESSED",
    };
    frame.sprite(skin, on, 14, 18);
    let auto = match (v.auto, pressed(Control::Auto)) {
        (false, false) => "EQ_AUTO_BUTTON",
        (false, true) => "EQ_AUTO_BUTTON_DEPRESSED",
        (true, false) => "EQ_AUTO_BUTTON_SELECTED",
        (true, true) => "EQ_AUTO_BUTTON_SELECTED_DEPRESSED",
    };
    frame.sprite(skin, auto, 40, 18);
    frame.sprite(skin, if pressed(Control::Presets) { "EQ_PRESETS_BUTTON_SELECTED" } else { "EQ_PRESETS_BUTTON" }, 217, 18);

    // The response graph: the preamp line, then the curve coloured by
    // height from EQ_GRAPH_LINE_COLORS.
    frame.sprite(skin, "EQ_GRAPH_BACKGROUND", 86, 17);
    let gy = |db: f32| 17 + ((12.0 - db.clamp(-12.0, 12.0)) / 24.0 * 18.0).round() as i32;
    frame.sprite(skin, "EQ_PREAMP_LINE", 86, gy(v.preamp));
    let colors: Vec<u32> = match skin.sprite("EQ_GRAPH_LINE_COLORS") {
        Some((img, r)) => (0..r.height).map(|i| {
            let p = img.get_pixel(r.x, r.y + i).0;
            argb([p[0], p[1], p[2]])
        }).collect(),
        None => vec![0xFF00_FF00; 19],
    };
    let ys = curve(&v.bands, 113);
    let mut last: Option<i32> = None;
    for (x, db) in ys.iter().enumerate() {
        let y = gy(*db);
        let (a, b) = match last {
            Some(l) => (l.min(y), l.max(y)),
            None => (y, y),
        };
        for yy in a..=b {
            let c = colors.get((yy - 17).clamp(0, 18) as usize).copied().unwrap_or(0xFF00_FF00);
            frame.put(86 + x as i32, yy, c);
        }
        last = Some(y);
    }

    slider(frame, skin, 21, v.preamp, pressed(Control::Preamp));
    for i in 0..10 {
        slider(frame, skin, band_x(i), v.bands[i], pressed(Control::Band(i)));
    }
}

/// Winamp's built-in presets (its winamp.q1): name, ten bands (dB).
pub const PRESETS: &[(&str, [f32; 10])] = &[
    ("Classical", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -4.9, -4.9, -4.9, -6.4]),
    ("Club", [0.0, 0.0, 1.9, 3.5, 3.5, 3.5, 1.9, 0.0, 0.0, 0.0]),
    ("Dance", [5.8, 4.3, 1.2, -0.4, -0.4, -4.1, -4.9, -4.9, -0.4, -0.4]),
    ("Full Bass", [5.8, 5.8, 5.8, 3.5, 0.8, -3.0, -5.6, -6.8, -7.1, -7.1]),
    ("Full Bass & Treble", [4.3, 3.5, 0.0, -4.9, -3.4, 0.8, 5.0, 6.6, 7.4, 7.4]),
    ("Full Treble", [-6.4, -6.4, -6.4, -3.0, 1.5, 6.6, 9.7, 9.7, 9.7, 10.5]),
    ("Laptop speakers/headphones", [2.7, 6.6, 3.1, -2.6, -1.9, 0.8, 2.7, 5.8, 7.7, 8.9]),
    ("Large hall", [6.2, 6.2, 3.5, 3.5, 0.0, -3.4, -3.4, -3.4, 0.0, 0.0]),
    ("Live", [-3.4, 0.0, 2.3, 3.1, 3.5, 3.5, 2.3, 1.5, 1.5, 1.2]),
    ("Party", [4.3, 4.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 4.3, 4.3]),
    ("Pop", [-1.5, 2.7, 4.3, 4.6, 3.1, -1.1, -1.9, -1.9, -1.5, -1.5]),
    ("Reggae", [0.0, 0.0, -0.8, -4.1, 0.0, 3.9, 3.9, 0.0, 0.0, 0.0]),
    ("Rock", [4.6, 2.7, -3.8, -5.2, -2.6, 2.3, 5.4, 6.6, 6.6, 6.6]),
    ("Ska", [-1.9, -3.4, -3.0, -0.8, 2.3, 3.5, 5.4, 5.8, 6.6, 5.8]),
    ("Soft", [2.7, 0.8, -1.1, -1.9, -1.1, 2.3, 5.0, 5.8, 6.6, 7.4]),
    ("Soft Rock", [2.3, 2.3, 1.2, -0.8, -3.0, -3.8, -2.6, -0.8, 1.5, 5.4]),
    ("Techno", [4.6, 3.5, 0.0, -3.8, -3.4, 0.0, 4.6, 5.8, 5.8, 5.4]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_and_hits() {
        assert_eq!(hit(80, 60, false), Some(Control::Band(0)));
        assert_eq!(hit(242, 60, false), Some(Control::Band(9)));
        assert_eq!(hit(25, 60, false), Some(Control::Preamp));
        assert_eq!(hit(20, 22, false), Some(Control::On));
        assert_eq!(db_at(SLIDER_Y + 5), 12.0);
        assert_eq!(db_at(SLIDER_Y + 5 + 51), -12.0);
        assert_eq!(db_at(SLIDER_Y + 5 + 25), 0.0);
        let flat = curve(&[3.0; 10], 113);
        assert!(flat.iter().all(|v| (v - 3.0).abs() < 1e-3));
    }

    #[test]
    fn draws() {
        let skin = Skin::default_skin();
        let mut f = Frame::new(1, 1);
        let v = View { on: true, preamp: -3.0, bands: PRESETS[12].1, focused: true, ..Default::default() };
        draw(&mut f, &skin, &v);
        assert_eq!((f.width, f.height), (WIDTH, HEIGHT));
        if let Ok(dir) = std::env::var("GAMP_SNAPSHOTS") {
            f.to_image().save(format!("{dir}/eq.png")).unwrap();
        }
    }

    #[test]
    fn shade_button_is_up_when_not_focused() {
        let skin = Skin::default_skin();
        let button = |v: &View| {
            let mut f = Frame::new(1, 1);
            draw(&mut f, &skin, v);
            (3..12).flat_map(|y| (254..263).map(move |x| (x, y))).map(|(x, y)| f.pixels[(y * WIDTH + x) as usize]).collect::<Vec<_>>()
        };
        let up = button(&View { focused: true, ..Default::default() });
        assert_eq!(button(&View::default()), up);
        assert_ne!(button(&View { pressed: Some(Control::Shade), ..Default::default() }), up);
    }
}
