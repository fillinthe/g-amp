//! Gamp's audio engine.
//!
//! A decoder thread reads the current track (any [`Input`]), turns it into
//! stereo, runs Winamp's equalizer, converts to the output's sample rate and
//! fills a lock-free ring buffer. The output callback drains it, applies
//! volume and balance, and copies what it plays to an analysis tap for the
//! visualizers. The next track can be queued so playback is gapless.

pub mod analysis;
pub mod decode;
pub mod eq;
pub mod output;
pub mod resample;

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

pub use decode::{Format, Input};
pub use eq::EqSettings;
pub use output::OutputKind;

use decode::Source;
use eq::Equalizer;
use output::Shared;
use resample::Resampler;

/// Half a second of audio in the ring: plenty for slow disks and networks,
/// short enough that the EQ answers quickly.
const RING_SECONDS: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Debug)]
pub enum Event {
    /// A track began to be heard (also after a gapless change).
    Started { id: u64, format: Format },
    /// A track finished playing to its end.
    Finished { id: u64 },
    Error { id: u64, message: String },
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: State,
    pub track: Option<u64>,
    pub position: f64,
    pub format: Option<Format>,
}

enum Cmd {
    Load { id: u64, input: Input, play: bool },
    Queue { id: u64, input: Input },
    Play,
    Pause,
    Stop,
    Seek(f64),
    Eq(EqSettings),
}

/// One stretch of the ring's timeline: track `id` starts being heard when
/// `start` frames have been read, at `base` seconds into it.
#[derive(Clone)]
struct Segment {
    start: u64,
    id: u64,
    base: f64,
    format: Format,
}

#[derive(Default)]
struct Timeline {
    segments: VecDeque<Segment>,
    /// Frames written once the last track ended (nothing queued).
    end: Option<u64>,
    state: Option<State>,
}

pub struct Player {
    cmd: Sender<Cmd>,
    shared: Arc<Shared>,
    timeline: Arc<Mutex<Timeline>>,
    events: Receiver<Event>,
    analysis: Mutex<analysis::Tap>,
    next_id: std::sync::atomic::AtomicU64,
    eq: Mutex<EqSettings>,
}

impl Player {
    pub fn new(kind: OutputKind) -> anyhow::Result<Player> {
        let shared = Arc::new(Shared::new());
        // Sized for the device rate once it's known; 192 kHz worst case.
        let probe_rate = 192_000f32;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new((probe_rate * 2.0 * RING_SECONDS) as usize);
        let (ana_tx, ana_rx) = rtrb::RingBuffer::<f32>::new(16384);
        let rate = output::start(kind, shared.clone(), consumer, ana_tx)?;
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let timeline = Arc::new(Mutex::new(Timeline::default()));
        let worker = Worker {
            shared: shared.clone(),
            timeline: timeline.clone(),
            ring: producer,
            ring_limit: (rate as f32 * 2.0 * RING_SECONDS) as usize,
            rate,
            cmds: cmd_rx,
            events: ev_tx,
            eq: Equalizer::new(rate, EqSettings::default()),
            current: None,
            queued: None,
            resampler: None,
            backlog: Vec::new(),
            written: 0,
            state: State::Stopped,
            announced: None,
        };
        std::thread::Builder::new().name("gamp-decoder".into()).spawn(move || worker.run())?;
        Ok(Player {
            cmd: cmd_tx,
            shared,
            timeline,
            events: ev_rx,
            analysis: Mutex::new(analysis::Tap::new(ana_rx)),
            next_id: 1.into(),
            eq: Mutex::new(EqSettings::default()),
        })
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Replaces what is playing. Returns the track's id for events.
    pub fn load(&self, input: Input, play: bool) -> u64 {
        let id = self.id();
        let _ = self.cmd.send(Cmd::Load { id, input, play });
        id
    }

    /// Plays `input` straight after the current track, with no gap.
    pub fn queue(&self, input: Input) -> u64 {
        let id = self.id();
        let _ = self.cmd.send(Cmd::Queue { id, input });
        id
    }

    pub fn play(&self) {
        let _ = self.cmd.send(Cmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.cmd.send(Cmd::Pause);
    }
    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.cmd.send(Cmd::Seek(secs));
    }

    /// 0..=1.
    pub fn set_volume(&self, v: f32) {
        self.shared.set_volume(v)
    }
    pub fn volume(&self) -> f32 {
        self.shared.volume()
    }
    /// -1 (left) ..= 1 (right).
    pub fn set_balance(&self, b: f32) {
        self.shared.set_balance(b)
    }
    pub fn balance(&self) -> f32 {
        self.shared.balance()
    }

    pub fn set_eq(&self, eq: EqSettings) {
        *self.eq.lock().unwrap() = eq;
        let _ = self.cmd.send(Cmd::Eq(eq));
    }
    pub fn eq(&self) -> EqSettings {
        *self.eq.lock().unwrap()
    }

    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    pub fn output_rate(&self) -> u32 {
        self.shared.rate.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> Status {
        let read = self.shared.frames_read.load(Ordering::Acquire);
        let rate = self.output_rate() as f64;
        let t = self.timeline.lock().unwrap();
        let state = t.state.unwrap_or(State::Stopped);
        let seg = t.segments.iter().rev().find(|s| s.start <= read).or(t.segments.front());
        match seg {
            Some(s) => {
                let mut position = s.base + read.saturating_sub(s.start) as f64 / rate;
                if let Some(d) = s.format.duration {
                    position = position.min(d);
                }
                Status { state, track: Some(s.id), position, format: Some(s.format.clone()) }
            }
            None => Status { state, track: None, position: 0.0, format: None },
        }
    }

    /// The latest `out.len()` samples heard (mono), for visualizers.
    pub fn waveform(&self, out: &mut [f32]) {
        self.analysis.lock().unwrap().latest(out)
    }
}

struct Worker {
    shared: Arc<Shared>,
    timeline: Arc<Mutex<Timeline>>,
    ring: rtrb::Producer<f32>,
    ring_limit: usize,
    rate: u32,
    cmds: Receiver<Cmd>,
    events: Sender<Event>,
    eq: Equalizer,
    current: Option<(u64, Source)>,
    queued: Option<(u64, Input)>,
    resampler: Option<(u32, Resampler)>,
    /// Converted samples that didn't fit in the ring yet.
    backlog: Vec<f32>,
    written: u64,
    state: State,
    /// The track whose Started event was sent.
    announced: Option<u64>,
}

impl Worker {
    fn run(mut self) {
        let mut decoded = Vec::new();
        loop {
            // Commands first, so controls feel instant.
            loop {
                match self.cmds.try_recv() {
                    Ok(cmd) => self.command(cmd),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => return,
                }
            }
            self.announce();
            if self.state != State::Playing || self.current.is_none() {
                if let Ok(cmd) = self.cmds.recv_timeout(Duration::from_millis(30)) {
                    self.command(cmd);
                }
                continue;
            }
            if !self.drain_backlog() || self.filled() >= self.ring_limit {
                if let Ok(cmd) = self.cmds.recv_timeout(Duration::from_millis(5)) {
                    self.command(cmd);
                }
                continue;
            }
            let Some((id, source)) = self.current.as_mut() else { continue };
            let id = *id;
            match source.next(&mut decoded) {
                Ok(Some(())) => {
                    let rate = source.info().rate;
                    self.eq.set_rate(rate);
                    self.eq.process(&mut decoded);
                    self.convert(rate, &decoded);
                }
                Ok(None) => self.track_done(id),
                Err(e) => {
                    let _ = self.events.send(Event::Error { id, message: e.to_string() });
                    self.track_done(id);
                }
            }
        }
    }

    fn filled(&self) -> usize {
        self.ring.buffer().capacity() - self.ring.slots()
    }

    fn convert(&mut self, rate: u32, samples: &[f32]) {
        let out = self.rate;
        let r = match &mut self.resampler {
            Some((r_rate, r)) if *r_rate == rate => r,
            slot => &mut slot.insert((rate, Resampler::new(rate, out))).1,
        };
        let backlog = &mut self.backlog;
        r.push(samples, |s| backlog.extend_from_slice(s));
        self.drain_backlog();
    }

    /// Moves converted audio into the ring; false while some is left.
    fn drain_backlog(&mut self) -> bool {
        if self.backlog.is_empty() {
            return true;
        }
        let room = self.ring_limit.saturating_sub(self.filled()).min(self.ring.slots()) & !1;
        let n = room.min(self.backlog.len());
        if n > 0 {
            if let Ok(mut chunk) = self.ring.write_chunk_uninit(n) {
                let (a, b) = chunk.as_mut_slices();
                for (dst, src) in a.iter_mut().chain(b.iter_mut()).zip(&self.backlog[..n]) {
                    dst.write(*src);
                }
                unsafe { chunk.commit_all() };
                self.written += (n / 2) as u64;
                self.backlog.drain(..n);
            }
        }
        self.backlog.is_empty()
    }

    fn start_segment(&mut self, id: u64, base: f64, format: Format) {
        let mut t = self.timeline.lock().unwrap();
        t.end = None;
        t.segments.push_back(Segment { start: self.written, id, base, format });
        // Keep only what the output hasn't passed.
        let read = self.shared.frames_read.load(Ordering::Acquire);
        while t.segments.len() > 1 && t.segments[1].start <= read {
            t.segments.pop_front();
        }
    }

    fn set_state(&mut self, s: State) {
        self.state = s;
        self.shared.paused.store(s != State::Playing, Ordering::Relaxed);
        self.timeline.lock().unwrap().state = Some(s);
    }

    /// Drops everything buffered, waiting for the output to let go of it.
    fn flush(&mut self) {
        self.backlog.clear();
        if let Some((_, r)) = &mut self.resampler {
            r.reset();
        }
        self.shared.flush.store(true, Ordering::Release);
        let start = Instant::now();
        while self.shared.flush.load(Ordering::Acquire) && start.elapsed() < Duration::from_millis(500) {
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.shared.flush.swap(false, Ordering::AcqRel) {
            // The output is gone or stuck: drop what it never took.
            self.shared.frames_read.store(self.written, Ordering::Release);
        }
        self.written = self.shared.frames_read.load(Ordering::Acquire).max(self.written);
        self.shared.frames_read.store(self.written, Ordering::Release);
    }

    fn open(&mut self, id: u64, input: Input) -> bool {
        match Source::open(input) {
            Ok(source) => {
                let format = source.info().clone();
                self.current = Some((id, source));
                self.start_segment(id, 0.0, format);
                true
            }
            Err(e) => {
                let _ = self.events.send(Event::Error { id, message: e.to_string() });
                false
            }
        }
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Load { id, input, play } => {
                self.flush();
                self.current = None;
                self.queued = None;
                self.timeline.lock().unwrap().segments.clear();
                self.announced = None;
                if self.open(id, input) {
                    self.set_state(if play { State::Playing } else { State::Paused });
                } else {
                    self.set_state(State::Stopped);
                }
            }
            Cmd::Queue { id, input } => {
                if self.current.is_none() {
                    if self.open(id, input) {
                        self.set_state(State::Playing);
                    }
                } else {
                    self.queued = Some((id, input));
                }
            }
            Cmd::Play => {
                if self.current.is_some() {
                    self.set_state(State::Playing);
                }
            }
            Cmd::Pause => {
                if self.state == State::Playing {
                    self.set_state(State::Paused);
                }
            }
            Cmd::Stop => {
                self.flush();
                self.queued = None;
                // Back to the start, ready to play again (a live stream is
                // dropped; it can't rewind).
                let rewound = match self.current.as_mut() {
                    Some((id, s)) if s.info().seekable => s.seek(0.0).ok().map(|_| (*id, s.info().clone())),
                    _ => None,
                };
                self.timeline.lock().unwrap().segments.clear();
                match rewound {
                    Some((id, format)) => self.start_segment(id, 0.0, format),
                    None => self.current = None,
                }
                self.announced = None;
                self.set_state(State::Stopped);
            }
            Cmd::Seek(secs) => {
                let Some((id, source)) = self.current.as_mut() else { return };
                let id = *id;
                if !source.info().seekable {
                    return;
                }
                let target = source.info().duration.map(|d| secs.min(d - 0.05)).unwrap_or(secs).max(0.0);
                let result = source.seek(target);
                let format = source.info().clone();
                // Whatever is queued next stays queued.
                self.flush();
                self.timeline.lock().unwrap().segments.clear();
                match result {
                    Ok(()) => self.start_segment(id, target, format),
                    Err(e) => {
                        let _ = self.events.send(Event::Error { id, message: e.to_string() });
                    }
                }
            }
            Cmd::Eq(eq) => self.eq.set(eq),
        }
    }

    fn track_done(&mut self, id: u64) {
        if let Some((_, r)) = &mut self.resampler {
            let backlog = &mut self.backlog;
            r.flush(|s| backlog.extend_from_slice(s));
            r.reset();
        }
        while !self.drain_backlog() {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.current = None;
        // Gapless: the next track's audio follows directly in the ring.
        if let Some((next, input)) = self.queued.take() {
            if self.open(next, input) {
                return;
            }
        }
        self.timeline.lock().unwrap().end = Some(self.written);
        let _ = id;
    }

    /// Sends Started when a track becomes audible, and Finished (and stops)
    /// once the last one has been heard to the end.
    fn announce(&mut self) {
        let read = self.shared.frames_read.load(Ordering::Acquire);
        let (heard, end) = {
            let t = self.timeline.lock().unwrap();
            let heard = t.segments.iter().rev().find(|s| s.start <= read).map(|s| (s.id, s.format.clone()));
            (heard, t.end)
        };
        if let Some((id, format)) = heard {
            if self.announced != Some(id) {
                if let Some(prev) = self.announced {
                    let _ = self.events.send(Event::Finished { id: prev });
                }
                self.announced = Some(id);
                let _ = self.events.send(Event::Started { id, format });
            }
        }
        if let Some(end) = end {
            if read >= end && self.current.is_none() && self.state == State::Playing {
                if let Some(id) = self.announced.take() {
                    let _ = self.events.send(Event::Finished { id });
                }
                let mut t = self.timeline.lock().unwrap();
                t.end = None;
                t.segments.clear();
                drop(t);
                self.set_state(State::Stopped);
            }
        }
    }
}
