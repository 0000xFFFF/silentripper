//! In-process silence detection (a native port of FFmpeg's `silencedetect` filter).
//!
//! Audio is decoded with libavcodec, converted to planar f32 with libswresample and
//! scanned directly. No filter graph and no subprocess are involved.
//!
//! Semantics match `silencedetect` with default options: a sample is "silent" when its
//! absolute value is below the noise threshold on *every* channel, and a run of silent
//! samples counts as silence once it lasts at least `min_duration` seconds.

use anyhow::{anyhow, Context, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::{codec, format, frame, media, software::resampling};
use ffmpeg::format::sample::Type as SampleType;
use ffmpeg::format::Sample;
use ffmpeg::util::channel_layout::ChannelLayout;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct Interval {
    pub start: f64,
    pub end: f64,
}

pub struct Detection {
    pub silences: Vec<Interval>,
    /// Timestamp (seconds) of the first audio sample.
    pub start: f64,
    /// Timestamp (seconds) just past the last audio sample.
    pub end: f64,
    /// Video keyframe timestamps (seconds), collected for free during the same demux pass.
    pub keyframes: Vec<f64>,
}

struct Detector {
    thr: f32,
    need: u64,
    rate: f64,
    run_start: u64,
    run_len: u64,
    total: u64,
    out: Vec<Interval>,
}

const CHUNK: usize = 256;

impl Detector {
    fn new(noise_db: f64, min_duration: f64, rate: u32) -> Self {
        Self {
            thr: 10f64.powf(noise_db / 20.0) as f32,
            need: ((min_duration * rate as f64).ceil() as u64).max(1),
            rate: rate as f64,
            run_start: 0,
            run_len: 0,
            total: 0,
            out: Vec::new(),
        }
    }

    #[inline]
    fn extend_silent(&mut self, n: u64) {
        if self.run_len == 0 {
            self.run_start = self.total;
        }
        self.run_len += n;
        self.total += n;
    }

    #[inline]
    fn loud_sample(&mut self) {
        if self.run_len >= self.need {
            self.out.push(Interval {
                start: self.run_start as f64 / self.rate,
                end: self.total as f64 / self.rate,
            });
        }
        self.run_len = 0;
        self.total += 1;
    }

    fn feed(&mut self, planes: &[&[f32]], n: usize) {
        let thr = self.thr;
        let mut i = 0;
        while i < n {
            let j = (i + CHUNK).min(n);
            // Vectorisable per-channel peak over the chunk.
            let mut loud = false;
            for p in planes {
                let peak = p[i..j].iter().fold(0f32, |m, &x| m.max(x.abs()));
                if peak >= thr {
                    loud = true;
                    break;
                }
            }
            if !loud {
                self.extend_silent((j - i) as u64);
            } else {
                for k in i..j {
                    if planes.iter().all(|p| p[k].abs() < thr) {
                        self.extend_silent(1);
                    } else {
                        self.loud_sample();
                    }
                }
            }
            i = j;
        }
    }

    fn finish(mut self) -> Vec<Interval> {
        if self.run_len >= self.need {
            self.out.push(Interval {
                start: self.run_start as f64 / self.rate,
                end: self.total as f64 / self.rate,
            });
        }
        self.out
    }
}

pub fn detect(path: &Path, noise_db: f64, min_duration: f64) -> Result<Detection> {
    let mut ictx = format::input(path).with_context(|| format!("cannot open {}", path.display()))?;
    let stream = ictx
        .streams()
        .best(media::Type::Audio)
        .ok_or_else(|| anyhow!("no audio stream found - nothing to detect silence in"))?;
    let a_idx = stream.index();
    let tb = f64::from(stream.time_base());
    let video = ictx.streams().best(media::Type::Video).map(|v| (v.index(), f64::from(v.time_base())));
    let mut keyframes: Vec<f64> = Vec::new();
    let mut dec = codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .audio()?;

    let rate = dec.rate();
    let layout = if dec.channel_layout().is_empty() {
        ChannelLayout::default(dec.channels() as i32)
    } else {
        dec.channel_layout()
    };
    let mut swr = resampling::Context::get(
        dec.format(),
        layout,
        rate,
        Sample::F32(SampleType::Planar),
        layout,
        rate,
    )?;

    let mut det = Detector::new(noise_db, min_duration, rate);
    let mut t0: Option<f64> = None;
    let mut decoded = frame::Audio::empty();
    let mut converted = frame::Audio::empty();

    let mut handle = |dec: &mut ffmpeg::decoder::Audio,
                      det: &mut Detector,
                      t0: &mut Option<f64>|
     -> Result<()> {
        while dec.receive_frame(&mut decoded).is_ok() {
            if t0.is_none() {
                *t0 = Some(decoded.pts().map(|p| p as f64 * tb).unwrap_or(0.0));
            }
            decoded.set_channel_layout(layout);
            converted = frame::Audio::empty();
            swr.run(&decoded, &mut converted)?;
            let n = converted.samples();
            if n == 0 {
                continue;
            }
            let planes: Vec<&[f32]> = (0..converted.planes())
                .map(|c| &converted.plane::<f32>(c)[..n])
                .collect();
            det.feed(&planes, n);
        }
        Ok(())
    };

    let mut packet = ffmpeg::Packet::empty();
    loop {
        match packet.read(&mut ictx) {
            Ok(()) => {
                if packet.stream() != a_idx {
                    if let Some((vi, vtb)) = video {
                        if packet.stream() == vi && packet.is_key() {
                            if let Some(t) = packet.dts().or(packet.pts()) {
                                keyframes.push(t as f64 * vtb);
                            }
                        }
                    }
                    continue;
                }
                if dec.send_packet(&packet).is_err() {
                    continue; // skip a corrupt packet, keep going
                }
                handle(&mut dec, &mut det, &mut t0)?;
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }
    let _ = dec.send_eof();
    handle(&mut dec, &mut det, &mut t0)?;

    let start = t0.unwrap_or(0.0);
    let total = det.total as f64 / rate as f64;
    let silences = det
        .finish()
        .into_iter()
        .map(|s| Interval { start: s.start + start, end: s.end + start })
        .collect();
    keyframes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Ok(Detection { silences, start, end: start + total, keyframes })
}
