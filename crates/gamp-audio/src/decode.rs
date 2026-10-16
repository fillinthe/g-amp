//! Turns an input into stereo f32 samples: anything Symphonia reads (MP3,
//! FLAC, AAC/M4A, Ogg Vorbis, WAV, AIFF, ALAC, MKV/WebM audio), or PCM that
//! is already decoded (Spotify).

use crossbeam_channel::Receiver;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Something to play.
pub enum Input {
    /// A file or stream to decode. `ext` ("mp3", "flac"...) helps detection;
    /// `duration` is used when the file doesn't say (fragmented MP4).
    Media { source: Box<dyn MediaSource>, ext: Option<String>, duration: Option<f64> },
    /// PCM decoded elsewhere: interleaved f32 chunks. `seek` jumps the
    /// producer; chunks after a seek must start at the new position.
    Pcm { rate: u32, channels: u16, duration: Option<f64>, chunks: Receiver<Vec<f32>>, seek: Option<Box<dyn FnMut(f64) + Send>> },
    /// Opened on the decoder thread when its turn comes (network sources:
    /// connecting must not hold up the window).
    Later(Box<dyn FnOnce() -> anyhow::Result<Input> + Send>),
}

impl Input {
    pub fn file(path: &std::path::Path) -> std::io::Result<Input> {
        let file = std::fs::File::open(path)?;
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        Ok(Input::Media { source: Box::new(file), ext, duration: None })
    }

    /// An endless or unseekable stream (radio).
    pub fn stream(reader: impl std::io::Read + Send + Sync + 'static, ext: Option<String>) -> Input {
        Input::Media { source: Box::new(ReadOnlySource::new(reader)), ext, duration: None }
    }
}

/// What the player knows about the track being played.
#[derive(Clone, Debug, Default)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
    pub duration: Option<f64>,
    pub kbps: Option<u32>,
    pub seekable: bool,
}

pub enum Source {
    Media(Media),
    Pcm(Pcm),
}

pub struct Media {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track: u32,
    info: Format,
    buf: Option<SampleBuffer<f32>>,
    /// After an accurate seek: frames still to drop before the target.
    skip: u64,
}

pub struct Pcm {
    chunks: Receiver<Vec<f32>>,
    seek: Option<Box<dyn FnMut(f64) + Send>>,
    info: Format,
}

impl Source {
    pub fn open(input: Input) -> anyhow::Result<Source> {
        match input {
            Input::Later(open) => Source::open(open()?),
            Input::Pcm { rate, channels, duration, chunks, seek } => Ok(Source::Pcm(Pcm {
                chunks,
                info: Format { rate, channels, duration, kbps: Some(rate * channels as u32 * 16 / 1000), seekable: seek.is_some() },
                seek,
            })),
            Input::Media { source, ext, duration: told } => {
                let seekable = source.is_seekable();
                let bytes = source.byte_len();
                let mss = MediaSourceStream::new(source, Default::default());
                let mut hint = Hint::new();
                if let Some(e) = ext.as_deref() {
                    hint.with_extension(e);
                }
                let probed = symphonia::default::get_probe().format(
                    &hint,
                    mss,
                    &FormatOptions { enable_gapless: true, ..Default::default() },
                    &MetadataOptions::default(),
                )?;
                let format = probed.format;
                let track = format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                    .ok_or_else(|| anyhow::anyhow!("no audio in this file"))?;
                let params = track.codec_params.clone();
                let decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default())?;
                let rate = params.sample_rate.unwrap_or(44100);
                let duration = params.n_frames.filter(|n| *n > 0).map(|n| n as f64 / rate as f64).or(told.filter(|d| *d > 0.0));
                let kbps = match (bytes, duration) {
                    (Some(b), Some(d)) if d > 0.5 => Some((b as f64 * 8.0 / d / 1000.0).round() as u32),
                    _ => params.bits_per_coded_sample.map(|b| b * rate * params.channels.map(|c| c.count() as u32).unwrap_or(2) / 1000),
                };
                let info = Format {
                    rate,
                    channels: params.channels.map(|c| c.count() as u16).unwrap_or(2),
                    duration,
                    kbps,
                    seekable: seekable && duration.is_some(),
                };
                Ok(Source::Media(Media { track: track.id, format, decoder, info, buf: None, skip: 0 }))
            }
        }
    }

    pub fn info(&self) -> &Format {
        match self {
            Source::Media(m) => &m.info,
            Source::Pcm(p) => &p.info,
        }
    }

    /// The next stereo samples; `Ok(None)` at the end.
    pub fn next(&mut self, out: &mut Vec<f32>) -> anyhow::Result<Option<()>> {
        out.clear();
        match self {
            Source::Pcm(p) => match p.chunks.recv() {
                Ok(chunk) => {
                    to_stereo(&chunk, p.info.channels as usize, out);
                    Ok(Some(()))
                }
                Err(_) => Ok(None),
            },
            Source::Media(m) => loop {
                let packet = match m.format.next_packet() {
                    Ok(p) => p,
                    Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                    Err(SymError::ResetRequired) => {
                        m.decoder.reset();
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                if packet.track_id() != m.track {
                    continue;
                }
                let decoded = match m.decoder.decode(&packet) {
                    Ok(d) => d,
                    // A damaged frame: skip it, as players do.
                    Err(SymError::DecodeError(_)) => continue,
                    Err(e) => return Err(e.into()),
                };
                let spec = *decoded.spec();
                if decoded.frames() == 0 {
                    continue;
                }
                // Some streams change rate or channels mid-way.
                m.info.rate = spec.rate;
                let ch = spec.channels.count();
                m.info.channels = ch as u16;
                let buf = match &mut m.buf {
                    Some(b) if b.capacity() >= decoded.capacity() * ch => b,
                    slot => slot.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
                };
                buf.copy_interleaved_ref(decoded);
                let mut samples = buf.samples();
                if m.skip > 0 {
                    let drop = (m.skip as usize).min(samples.len() / ch);
                    samples = &samples[drop * ch..];
                    m.skip -= drop as u64;
                    if samples.is_empty() {
                        continue;
                    }
                }
                to_stereo(samples, ch, out);
                return Ok(Some(()));
            },
        }
    }

    /// Jumps to `secs`, to the sample.
    pub fn seek(&mut self, secs: f64) -> anyhow::Result<()> {
        match self {
            Source::Pcm(p) => {
                if let Some(f) = p.seek.as_mut() {
                    // Drop what was produced for the old position.
                    while p.chunks.try_recv().is_ok() {}
                    f(secs);
                }
                Ok(())
            }
            Source::Media(m) => {
                if !m.info.seekable {
                    anyhow::bail!("this stream can't seek");
                }
                let secs = secs.max(0.0);
                let to = m.format.seek(SeekMode::Accurate, SeekTo::Time { time: Time::from(secs), track_id: Some(m.track) })?;
                m.decoder.reset();
                m.skip = to.required_ts.saturating_sub(to.actual_ts);
                Ok(())
            }
        }
    }
}

/// Interleaved audio with any channel count to stereo.
pub fn to_stereo(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => out.extend(samples.iter().flat_map(|s| [*s, *s])),
        2 => out.extend_from_slice(samples),
        n => {
            // L, R, and (5.1 and up) a share of the centre.
            for f in samples.chunks_exact(n) {
                let c = if n >= 3 { f[2] * 0.7071 } else { 0.0 };
                out.push(f[0] + c);
                out.push(f[1] + c);
            }
        }
    }
}
