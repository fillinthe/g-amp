//! Winamp's 10-band equalizer: one peaking filter per band at the same
//! centre frequencies, ±12 dB each, plus a ±12 dB preamp.

/// Winamp's band centres, in Hz.
pub const BANDS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];

/// What the equalizer window sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    pub on: bool,
    /// dB, -12..=12.
    pub preamp: f32,
    /// dB, -12..=12, one per entry of [`BANDS`].
    pub bands: [f32; 10],
}

impl Default for EqSettings {
    fn default() -> Self {
        EqSettings { on: true, preamp: 0.0, bands: [0.0; 10] }
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    // Per channel state (stereo).
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    /// RBJ cookbook peaking filter.
    fn peaking(freq: f32, gain_db: f32, q: f32, rate: f32) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * freq / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha / a) / a0,
            ..Default::default()
        }
    }

    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        // Transposed direct form II: good numerically in f32.
        let y = self.b0 * x + self.z1[ch];
        self.z1[ch] = self.b1 * x - self.a1 * y + self.z2[ch];
        self.z2[ch] = self.b2 * x - self.a2 * y;
        y
    }
}

/// The equalizer for one stream of interleaved stereo samples.
pub struct Equalizer {
    rate: f32,
    settings: EqSettings,
    filters: Vec<Biquad>,
    preamp: f32,
}

impl Equalizer {
    pub fn new(rate: u32, settings: EqSettings) -> Equalizer {
        let mut eq = Equalizer { rate: rate as f32, settings, filters: Vec::new(), preamp: 1.0 };
        eq.rebuild();
        eq
    }

    pub fn settings(&self) -> EqSettings {
        self.settings
    }

    /// New settings; filter memory carries over so there is no click.
    pub fn set(&mut self, settings: EqSettings) {
        if settings == self.settings {
            return;
        }
        let old = std::mem::take(&mut self.filters);
        self.settings = settings;
        self.rebuild();
        for (new, old) in self.filters.iter_mut().zip(old) {
            new.z1 = old.z1;
            new.z2 = old.z2;
        }
    }

    pub fn set_rate(&mut self, rate: u32) {
        if rate as f32 != self.rate {
            self.rate = rate as f32;
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        let s = self.settings;
        self.preamp = 10f32.powf(s.preamp.clamp(-12.0, 12.0) / 20.0);
        // About an octave wide, like Winamp's bands; bands near the top of
        // a low sample rate are left out.
        self.filters = BANDS
            .iter()
            .zip(s.bands)
            .filter(|(f, _)| **f < self.rate * 0.45)
            .map(|(f, g)| Biquad::peaking(*f, g.clamp(-12.0, 12.0), 1.2, self.rate))
            .collect();
    }

    /// Filters interleaved stereo in place.
    pub fn process(&mut self, stereo: &mut [f32]) {
        if !self.settings.on {
            return;
        }
        let flat = self.settings.preamp == 0.0 && self.settings.bands.iter().all(|g| *g == 0.0);
        if flat {
            return;
        }
        for frame in stereo.chunks_exact_mut(2) {
            for (ch, s) in frame.iter_mut().enumerate() {
                let mut x = *s * self.preamp;
                for f in &mut self.filters {
                    x = f.run(ch, x);
                }
                *s = x;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level of a sine at `freq` after the EQ, in dB relative to its input.
    fn gain_at(freq: f32, settings: EqSettings) -> f32 {
        let rate = 44100;
        let mut eq = Equalizer::new(rate, settings);
        let n = rate as usize;
        let mut buf: Vec<f32> = (0..n).flat_map(|i| {
            let v = (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * 0.25;
            [v, v]
        }).collect();
        eq.process(&mut buf);
        // Skip the settling time.
        let tail = &buf[n..];
        let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * (rms / (0.25 / 2f32.sqrt())).log10()
    }

    #[test]
    fn boosts_and_cuts_the_right_band() {
        let mut s = EqSettings::default();
        s.bands[4] = 12.0; // 1 kHz
        assert!((gain_at(1000.0, s) - 12.0).abs() < 1.0, "{}", gain_at(1000.0, s));
        assert!(gain_at(60.0, s).abs() < 1.0);
        s.bands[4] = -12.0;
        assert!((gain_at(1000.0, s) + 12.0).abs() < 1.5);
    }

    #[test]
    fn flat_and_off_change_nothing() {
        assert!(gain_at(440.0, EqSettings::default()).abs() < 0.01);
        let mut s = EqSettings::default();
        s.bands = [12.0; 10];
        s.on = false;
        assert!(gain_at(440.0, s).abs() < 0.01);
    }

    #[test]
    fn preamp() {
        let s = EqSettings { preamp: -6.0, ..Default::default() };
        assert!((gain_at(440.0, s) + 6.0).abs() < 0.2);
    }
}
