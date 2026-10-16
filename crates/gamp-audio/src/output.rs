//! The sound card side: a callback that takes samples from the ring
//! buffer, applies volume and balance, and feeds the analysis tap. It never
//! allocates or locks; settings arrive through atomics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

/// Shared between the player, the decoder thread and the output.
pub struct Shared {
    volume: AtomicU32,
    balance: AtomicU32,
    pub paused: AtomicBool,
    /// Set by the decoder to drop everything buffered (seek, stop).
    pub flush: AtomicBool,
    /// Frames taken from the ring (played or flushed).
    pub frames_read: AtomicU64,
    pub underruns: AtomicU64,
    pub rate: AtomicU32,
}

impl Shared {
    pub fn new() -> Shared {
        Shared {
            volume: AtomicU32::new(0.8f32.to_bits()),
            balance: AtomicU32::new(0f32.to_bits()),
            paused: AtomicBool::new(true),
            flush: AtomicBool::new(false),
            frames_read: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            rate: AtomicU32::new(48000),
        }
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed)
    }
    pub fn balance(&self) -> f32 {
        f32::from_bits(self.balance.load(Ordering::Relaxed))
    }
    pub fn set_balance(&self, b: f32) {
        self.balance.store(b.clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed)
    }
}

/// Where the sound goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OutputKind {
    /// The system's default output device.
    Device,
    /// Nowhere, at the pace of a real device (tests and headless runs).
    Null,
    /// Nowhere, as fast as possible (tests).
    NullFast,
}

struct Tap {
    shared: Arc<Shared>,
    ring: rtrb::Consumer<f32>,
    analysis: rtrb::Producer<f32>,
}

impl Tap {
    /// Fills `out` (interleaved, `channels` per frame) with up to its length
    /// in frames; returns frames that held music.
    #[inline]
    fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T], channels: usize) {
        let s = &self.shared;
        if s.flush.load(Ordering::Acquire) {
            let n = self.ring.slots();
            if let Ok(chunk) = self.ring.read_chunk(n) {
                chunk.commit_all();
            }
            s.frames_read.fetch_add((n / 2) as u64, Ordering::AcqRel);
            s.flush.store(false, Ordering::Release);
        }
        let silence = T::from_sample(0.0f32);
        if s.paused.load(Ordering::Relaxed) {
            out.iter_mut().for_each(|o| *o = silence);
            return;
        }
        let vol = s.volume();
        let vol = vol * vol; // the slider feels linear to the ear
        let bal = s.balance();
        let (gl, gr) = (vol * (1.0 - bal.max(0.0)), vol * (1.0 + bal.min(0.0)));
        let mut played = 0u64;
        let mut starved = false;
        for frame in out.chunks_mut(channels) {
            let (l, r) = match (self.ring.pop(), self.ring.pop()) {
                (Ok(l), Ok(r)) => {
                    played += 1;
                    (l, r)
                }
                _ => {
                    starved = true;
                    (0.0, 0.0)
                }
            };
            let _ = self.analysis.push((l + r) * 0.5);
            let (l, r) = ((l * gl).clamp(-1.0, 1.0), (r * gr).clamp(-1.0, 1.0));
            if channels == 1 {
                frame[0] = T::from_sample((l + r) * 0.5);
            } else {
                frame[0] = T::from_sample(l);
                frame[1] = T::from_sample(r);
                frame[2..].iter_mut().for_each(|o| *o = silence);
            }
        }
        if played > 0 {
            s.frames_read.fetch_add(played, Ordering::AcqRel);
            if starved {
                s.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Starts the output; returns its sample rate. The stream lives on its
/// own thread for the life of the program.
pub fn start(kind: OutputKind, shared: Arc<Shared>, ring: rtrb::Consumer<f32>, analysis: rtrb::Producer<f32>) -> anyhow::Result<u32> {
    let tap = Tap { shared: shared.clone(), ring, analysis };
    if kind != OutputKind::Device {
        let rate = 48000;
        shared.rate.store(rate, Ordering::Relaxed);
        std::thread::Builder::new().name("gamp-null-output".into()).spawn(move || null_output(tap, rate, kind == OutputKind::NullFast))?;
        return Ok(rate);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("gamp-output".into()).spawn(move || {
        let result = (|| -> anyhow::Result<(cpal::Stream, u32)> {
            let host = cpal::default_host();
            let device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("no sound output device"))?;
            let config = device.default_output_config()?;
            let rate = config.sample_rate();
            let channels = config.channels() as usize;
            let mut stream_config: cpal::StreamConfig = config.config();
            // About 20 ms per callback: responsive, gentle on the CPU.
            stream_config.buffer_size = cpal::BufferSize::Default;
            macro_rules! build {
                ($t:ty) => {{
                    let mut tap = tap;
                    device.build_output_stream(
                        stream_config,
                        move |data: &mut [$t], _| tap.fill(data, channels),
                        |e| log::warn!("audio output: {e}"),
                        None,
                    )?
                }};
            }
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => build!(f32),
                cpal::SampleFormat::I16 => build!(i16),
                cpal::SampleFormat::U16 => build!(u16),
                cpal::SampleFormat::I32 => build!(i32),
                cpal::SampleFormat::F64 => build!(f64),
                other => anyhow::bail!("unsupported output format {other:?}"),
            };
            stream.play()?;
            Ok((stream, rate))
        })();
        match result {
            Ok((stream, rate)) => {
                let _ = tx.send(Ok(rate));
                // Keep the stream alive.
                let _keep = stream;
                loop {
                    std::thread::park();
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        }
    })?;
    let rate = rx.recv()??;
    shared.rate.store(rate, Ordering::Relaxed);
    Ok(rate)
}

fn null_output(mut tap: Tap, rate: u32, fast: bool) {
    let block = (rate / 100) as usize; // 10 ms
    let mut buf = vec![0f32; block * 2];
    loop {
        tap.fill(&mut buf, 2);
        std::thread::sleep(if fast { Duration::from_micros(200) } else { Duration::from_millis(10) });
    }
}
