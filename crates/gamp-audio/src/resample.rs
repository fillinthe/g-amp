//! Sample-rate conversion to the output device's rate (FFT resampler:
//! clean and cheap). Passes audio straight through when rates match.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

const CHUNK: usize = 1024;

pub struct Resampler {
    inner: Option<Fft<f32>>,
    pending: Vec<f32>,
    out: Vec<f32>,
}

impl Resampler {
    /// Stereo, `from` Hz to `to` Hz.
    pub fn new(from: u32, to: u32) -> Resampler {
        let inner = (from != to).then(|| Fft::<f32>::new(from as usize, to as usize, CHUNK, 2, FixedSync::Input).expect("resampler"));
        let cap = inner.as_ref().map(|r| r.output_frames_max() * 2).unwrap_or(0);
        Resampler { inner, pending: Vec::new(), out: vec![0.0; cap] }
    }

    pub fn is_passthrough(&self) -> bool {
        self.inner.is_none()
    }

    /// Feeds interleaved stereo; `emit` gets converted audio as it is ready.
    pub fn push(&mut self, input: &[f32], mut emit: impl FnMut(&[f32])) {
        let Some(r) = self.inner.as_mut() else {
            emit(input);
            return;
        };
        self.pending.extend_from_slice(input);
        let mut used = 0;
        loop {
            let need = r.input_frames_next();
            let have = (self.pending.len() - used) / 2;
            if have < need {
                break;
            }
            let inp = InterleavedSlice::new(&self.pending[used..], 2, have).expect("input");
            let cap = self.out.len() / 2;
            let mut outp = InterleavedSlice::new_mut(&mut self.out, 2, cap).expect("output");
            let (n_in, n_out) = r.process_into_buffer(&inp, &mut outp, None::<&Indexing>).expect("resample");
            used += n_in * 2;
            emit(&self.out[..n_out * 2]);
        }
        self.pending.drain(..used);
    }

    /// The last partial chunk, at the end of a stream.
    pub fn flush(&mut self, mut emit: impl FnMut(&[f32])) {
        let Some(r) = self.inner.as_mut() else { return };
        let have = self.pending.len() / 2;
        if have == 0 {
            return;
        }
        // The resampler reads a whole chunk; the tail beyond `have` is silence.
        let need = r.input_frames_next().max(have);
        self.pending.resize(need * 2, 0.0);
        let inp = InterleavedSlice::new(&self.pending, 2, need).expect("input");
        let cap = self.out.len() / 2;
        let mut outp = InterleavedSlice::new_mut(&mut self.out, 2, cap).expect("output");
        let idx = Indexing { input_offset: 0, output_offset: 0, partial_len: Some(have), active_channels_mask: None };
        if let Ok((_, n_out)) = r.process_into_buffer(&inp, &mut outp, Some(&idx)) {
            emit(&self.out[..n_out * 2]);
        }
        self.pending.clear();
    }

    /// After a seek: forget what was buffered.
    pub fn reset(&mut self) {
        self.pending.clear();
        if let Some(r) = self.inner.as_mut() {
            r.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_length_and_keeps_the_tone() {
        let mut r = Resampler::new(44100, 48000);
        let input: Vec<f32> = (0..44100).flat_map(|i| {
            let v = (std::f32::consts::TAU * 1000.0 * i as f32 / 44100.0).sin() * 0.5;
            [v, v]
        }).collect();
        let mut out = Vec::new();
        for chunk in input.chunks(2 * 700) {
            r.push(chunk, |o| out.extend_from_slice(o));
        }
        r.flush(|o| out.extend_from_slice(o));
        let frames = out.len() / 2;
        assert!((frames as i64 - 48000).abs() < 2100, "{frames}");
        // Count zero crossings in the middle: 1 kHz stays 1 kHz.
        let mid = &out[20000..20000 + 48000 / 2 * 2];
        let left: Vec<f32> = mid.iter().step_by(2).copied().collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((crossings as i64 - 500).abs() <= 3, "{crossings}");
    }

    #[test]
    fn same_rate_passes_through() {
        let mut r = Resampler::new(48000, 48000);
        assert!(r.is_passthrough());
        let mut out = Vec::new();
        r.push(&[0.1, 0.2], |o| out.extend_from_slice(o));
        assert_eq!(out, vec![0.1, 0.2]);
    }
}
