mod gpu;
mod remux;
mod segments;
mod silence;
mod transcode;

use anyhow::{bail, Result};
use clap::{ArgGroup, Parser, ValueEnum};
use ffmpeg_next as ffmpeg;
use gpu::Codec;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;
use transcode::{GpuMode, Opts};

#[derive(Copy, Clone, ValueEnum)]
enum CodecArg {
    H264,
    Hevc,
}

/// Remove silent parts from video. Pure libav: no ffmpeg/ffprobe subprocesses.
#[derive(Parser)]
#[command(version, about)]
#[command(group(ArgGroup::new("gpumode").args(["gpu", "gpu_nvidia", "gpu_amd"])))]
struct Args {
    /// Input video file
    filename: PathBuf,

    /// Output file (default: <input>_ripped.<ext>)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Minimum silence duration in seconds to cut
    #[arg(short = 'd', long = "min-duration-silence", value_name = "sec", default_value_t = 1.0)]
    min_duration_silence: f64,

    /// Minimum duration of each sounded clip to keep, in seconds
    #[arg(short = 'm', long = "min-duration-sounded", value_name = "sec", default_value_t = 0.0)]
    min_duration_sounded: f64,

    /// Noise level in dB below which audio counts as silence
    #[arg(short = 'n', long = "noise", value_name = "dB", default_value_t = -30.0, allow_negative_numbers = true)]
    noise: f64,

    /// Keep this many seconds of padding around sounded clips
    #[arg(long, value_name = "sec", default_value_t = 0.0)]
    pad: f64,

    /// Stream copy (no re-encode): fastest, cuts land on keyframes
    #[arg(short = 'c', long)]
    copy: bool,

    /// Prompt before each action
    #[arg(short = 'p', long)]
    pause: bool,

    /// Auto-detect the GPU and use its hardware encoder
    #[arg(long)]
    gpu: bool,

    /// Use NVIDIA NVENC
    #[arg(long = "gpu-nvidia")]
    gpu_nvidia: bool,

    /// Use AMD (VAAPI on Linux, AMF on Windows)
    #[arg(long = "gpu-amd")]
    gpu_amd: bool,

    /// Video codec to encode with
    #[arg(long, value_enum, default_value_t = CodecArg::H264)]
    codec: CodecArg,

    /// Quality (CRF for CPU, CQ/QP for GPU); lower = better
    #[arg(long, default_value_t = 23)]
    crf: u32,

    /// x264/x265 preset (CPU encoder only)
    #[arg(long, default_value = "veryfast")]
    preset: String,

    /// Audio bitrate in kbit/s
    #[arg(long, default_value_t = 192)]
    abitrate: usize,
}

fn pause(on: bool, msg: &str) {
    if on {
        eprint!("{msg} [Enter] ");
        let _ = io::stderr().flush();
        let mut s = String::new();
        let _ = io::stdin().read_line(&mut s);
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let a = Args::parse();
    ffmpeg::init()?;
    ffmpeg::log::set_level(if std::env::var_os("SILENTRIPPER_LOG").is_some() {
        ffmpeg::log::Level::Info
    } else {
        ffmpeg::log::Level::Error
    });

    if !a.filename.exists() {
        bail!("{} does not exist", a.filename.display());
    }
    let output = a.output.clone().unwrap_or_else(|| {
        let stem = a.filename.file_stem().unwrap_or_default().to_string_lossy();
        let ext = a.filename.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "mp4".into());
        a.filename.with_file_name(format!("{stem}_ripped.{ext}"))
    });

    // 1. detect
    pause(a.pause, "detect silence?");
    let t = Instant::now();
    let det = silence::detect(&a.filename, a.noise, a.min_duration_silence)?;
    eprintln!(
        "detected {} silent intervals in {:.2}s (noise {} dB, min {}s)",
        det.silences.len(),
        t.elapsed().as_secs_f64(),
        a.noise,
        a.min_duration_silence
    );

    let mut segs = segments::sounded(&det, a.min_duration_sounded, a.pad);
    if segs.is_empty() {
        bail!("nothing but silence at this threshold - try a lower -n (e.g. -n -40)");
    }

    // 2. cut
    pause(a.pause, "write output?");
    let t = Instant::now();
    if a.copy {
        segs = segments::snap_to_keyframes(&segs, &det.keyframes);
        eprintln!("keeping {} clips ({:.1}s of {:.1}s), stream copy", segs.len(), segments::total(&segs), det.end - det.start);
        remux::run(&a.filename, &output, &segs)?;
    } else {
        eprintln!("keeping {} clips ({:.1}s of {:.1}s)", segs.len(), segments::total(&segs), det.end - det.start);
        let mode = if a.gpu_nvidia {
            GpuMode::Nvidia
        } else if a.gpu_amd {
            GpuMode::Amd
        } else if a.gpu {
            GpuMode::Auto
        } else {
            GpuMode::Off
        };
        let o = Opts {
            codec: match a.codec {
                CodecArg::H264 => Codec::H264,
                CodecArg::Hevc => Codec::Hevc,
            },
            gpu: mode,
            crf: a.crf,
            preset: a.preset.clone(),
            audio_bitrate: a.abitrate * 1000,
        };
        let backend = transcode::run(&a.filename, &output, &det, &segs, &o)?;
        eprintln!("encoder: {}", backend.encoder_name(o.codec));
    }
    eprintln!("wrote {} in {:.2}s", output.display(), t.elapsed().as_secs_f64());
    Ok(())
}
