//! The main window's little visualizer (76×16): Winamp's spectrum
//! analyzer with falling peaks, or its oscilloscope.

use crate::frame::{Frame, argb};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VisMode {
    #[default]
    Analyzer,
    Oscilloscope,
    Off,
}

/// Bar heights and peaks that fall like Winamp's.
#[derive(Clone)]
pub struct Analyzer {
    pub bars: [f32; 19],
    pub peaks: [f32; 19],
    peak_hold: [u8; 19],
}

impl Default for Analyzer {
    fn default() -> Self {
        Analyzer { bars: [0.0; 19], peaks: [0.0; 19], peak_hold: [0; 19] }
    }
}

impl Analyzer {
    /// New levels (0..1); bars rise at once and fall gently.
    pub fn update(&mut self, levels: &[f32]) {
        for i in 0..19 {
            let v = levels.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            self.bars[i] = if v > self.bars[i] { v } else { (self.bars[i] - 0.06).max(v) };
            if self.bars[i] >= self.peaks[i] {
                self.peaks[i] = self.bars[i];
                self.peak_hold[i] = 6;
            } else if self.peak_hold[i] > 0 {
                self.peak_hold[i] -= 1;
            } else {
                self.peaks[i] = (self.peaks[i] - 0.03).max(0.0);
            }
        }
    }

    pub fn decay(&mut self) {
        self.update(&[0.0; 19]);
    }
}

/// Draws the visualizer at `(x, y)` with the skin's VISCOLOR colours.
pub fn draw(frame: &mut Frame, colors: &[[u8; 3]], x: i32, y: i32, mode: VisMode, analyzer: &Analyzer, wave: &[f32]) {
    let c = |i: usize| argb(colors.get(i).copied().unwrap_or([0, 0, 0]));
    frame.fill(x, y, 76, 16, c(0));
    // The dotted grid.
    for yy in (1..16).step_by(2) {
        for xx in (1..76).step_by(2) {
            frame.put(x + xx, y + yy, c(1));
        }
    }
    match mode {
        VisMode::Off => {}
        VisMode::Analyzer => {
            for (i, (bar, peak)) in analyzer.bars.iter().zip(analyzer.peaks).enumerate() {
                let h = (bar * 16.0).round() as i32;
                for row in 0..h {
                    // Colours 2..17 run top (row 0) to bottom.
                    let screen_y = 15 - row;
                    for dx in 0..3 {
                        frame.put(x + i as i32 * 4 + dx, y + screen_y, c(2 + screen_y as usize));
                    }
                }
                let p = (peak * 16.0).round() as i32;
                if p > 0 {
                    for dx in 0..3 {
                        frame.put(x + i as i32 * 4 + dx, y + 16 - p, c(23));
                    }
                }
            }
        }
        VisMode::Oscilloscope => {
            if wave.is_empty() {
                return;
            }
            let mut last: Option<i32> = None;
            for xx in 0..76 {
                let s = wave[xx * wave.len() / 76].clamp(-1.0, 1.0);
                let yy = (7.5 - s * 7.5).round() as i32;
                let col = c(18 + ((yy - 8).unsigned_abs() as usize * 5 / 9).min(4));
                let (a, b) = match last {
                    Some(l) => (l.min(yy), l.max(yy)),
                    None => (yy, yy),
                };
                for v in a..=b {
                    frame.put(x + xx as i32, y + v, col);
                }
                last = Some(yy);
            }
        }
    }
}
