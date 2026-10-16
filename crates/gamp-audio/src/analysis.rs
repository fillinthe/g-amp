//! What the visualizers read: the most recent samples heard (mono), and a
//! spectrum in Winamp's style.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// Keeps the last few thousand samples from the output's tap.
pub struct Tap {
    rx: rtrb::Consumer<f32>,
    ring: Vec<f32>,
    pos: usize,
}

const KEEP: usize = 4096;

impl Tap {
    pub fn new(rx: rtrb::Consumer<f32>) -> Tap {
        Tap { rx, ring: vec![0.0; KEEP], pos: 0 }
    }

    fn pull(&mut self) {
        while let Ok(s) = self.rx.pop() {
            self.ring[self.pos] = s;
            self.pos = (self.pos + 1) % KEEP;
        }
    }

    /// The newest `out.len()` samples, oldest first.
    pub fn latest(&mut self, out: &mut [f32]) {
        self.pull();
        let n = out.len().min(KEEP);
        let start = (self.pos + KEEP - n) % KEEP;
        for (i, o) in out.iter_mut().take(n).enumerate() {
            *o = self.ring[(start + i) % KEEP];
        }
    }
}

/// Magnitude spectrum of a block of samples (Hann window).
pub struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex32>,
}

impl Spectrum {
    pub fn new(size: usize) -> Spectrum {
        let fft = FftPlanner::new().plan_fft_forward(size);
        let window = (0..size).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos()).collect();
        Spectrum { fft, window, buf: vec![Complex32::default(); size] }
    }

    pub fn size(&self) -> usize {
        self.window.len()
    }

    /// `samples.len()` must equal the size; `out` gets size/2 magnitudes.
    pub fn compute(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        for ((b, s), w) in self.buf.iter_mut().zip(samples).zip(&self.window) {
            *b = Complex32::new(s * w, 0.0);
        }
        self.fft.process(&mut self.buf);
        let n = self.buf.len();
        out.clear();
        out.extend(self.buf[..n / 2].iter().map(|c| c.norm() * 2.0 / n as f32));
    }
}

/// Folds a spectrum into `bars` log-spaced bands (0..1), the way Winamp's
/// analyzer looks: lows on the left, each bar about a third of an octave.
pub fn bars(spectrum: &[f32], rate: u32, bars: usize, out: &mut Vec<f32>) {
    out.clear();
    let n = spectrum.len();
    if n == 0 {
        out.resize(bars, 0.0);
        return;
    }
    let hz_per_bin = rate as f32 / 2.0 / n as f32;
    let (lo, hi) = (40f32, 16000f32.min(rate as f32 / 2.0));
    for b in 0..bars {
        let f0 = lo * (hi / lo).powf(b as f32 / bars as f32);
        let f1 = lo * (hi / lo).powf((b + 1) as f32 / bars as f32);
        let i0 = ((f0 / hz_per_bin) as usize).min(n - 1);
        let i1 = ((f1 / hz_per_bin) as usize).clamp(i0 + 1, n);
        let peak = spectrum[i0..i1].iter().fold(0f32, |m, v| m.max(*v));
        // dB scale, -60..0 dB → 0..1.
        let db = 20.0 * (peak + 1e-9).log10();
        out.push(((db + 60.0) / 60.0).clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lights_its_bar() {
        let rate = 44100;
        let mut s = Spectrum::new(1024);
        let samples: Vec<f32> = (0..1024).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / rate as f32).sin() * 0.8).collect();
        let mut spec = Vec::new();
        s.compute(&samples, &mut spec);
        let mut b = Vec::new();
        bars(&spec, rate, 19, &mut b);
        let loudest = b.iter().enumerate().max_by(|a, c| a.1.total_cmp(c.1)).unwrap().0;
        // 1 kHz sits in the middle of 40 Hz..16 kHz on a log scale.
        assert!((8..=11).contains(&loudest), "{loudest} {b:?}");
    }
}
