mod gpu;
mod remux;
mod segments;
mod silence;
mod transcode;
mod ui;

use anyhow::{bail, Result};
use clap::{ArgGroup, Parser, ValueEnum};
use ffmpeg_next as ffmpeg;
use gpu::Codec;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;
use transcode::{GpuMode, Opts};
use ui::{hms, human, ago};

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
    ui::start_clock();
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

    let mode = if a.gpu_nvidia {
        GpuMode::Nvidia
    } else if a.gpu_amd {
        GpuMode::Amd
    } else if a.gpu {
        GpuMode::Auto
    } else {
        GpuMode::Off
    };

    eprintln!("silentripper {}", env!("CARGO_PKG_VERSION"));
    eprintln!("  input : {}", a.filename.display());
    eprintln!("  output: {}", output.display());
    eprintln!(
        "  mode  : {}",
        if a.copy {
            "stream copy (no re-encode, cuts land on keyframes)".to_string()
        } else {
            format!("re-encode ({})", match mode {
                GpuMode::Off => "CPU",
                GpuMode::Auto => "GPU auto-detect",
                GpuMode::Nvidia => "GPU NVIDIA",
                GpuMode::Amd => "GPU AMD",
            })
        }
    );
    eprintln!();

    // [1/3] detect
    pause(a.pause, "detect silence?");
    ui::stage(
        1,
        3,
        &format!(
            "Detecting silent intervals (noise floor {} dB, silence of at least {}s)...",
            a.noise, a.min_duration_silence
        ),
    );
    let t = Instant::now();
    let mut prog = ui::Progress::new("Detecting", 0.0);
    let det = silence::detect(&a.filename, a.noise, a.min_duration_silence, &mut prog)?;
    prog.finish();
    ui::info(&format!(
        "found {} silent interval(s) in {}",
        det.silences.len(),
        human(t.elapsed().as_secs_f64())
    ));
    eprintln!();

    // [2/3] plan
    ui::stage(2, 3, "Planning cuts...");
    let mut segs = segments::sounded(&det, a.min_duration_sounded, a.pad);
    if segs.is_empty() {
        bail!("nothing but silence at this threshold - try a lower -n (e.g. -n -40)");
    }
    if a.copy {
        ui::info("snapping clip starts back to keyframes...");
        segs = segments::snap_to_keyframes(&segs, &det.keyframes);
    }
    let source = (det.end - det.start).max(0.0);
    let kept = segments::total(&segs);
    let cut = (source - kept).max(0.0);
    let pct = |x: f64| if source > 0.0 { x / source * 100.0 } else { 0.0 };
    ui::info(&format!("source length : {}", hms(source)));
    ui::info(&format!("cut (silent)  : {}  ({:.1}%)", hms(cut), pct(cut)));
    ui::info(&format!("kept (sounded): {}  ({:.1}%) in {} clip(s)", hms(kept), pct(kept), segs.len()));
    eprintln!();

    // [3/3] write
    pause(a.pause, "write output?");
    let t = Instant::now();
    if a.copy {
        ui::stage(3, 3, "Cutting and copying streams (no re-encode)...");
        let mut prog = ui::Progress::new("Copying", kept);
        remux::run(&a.filename, &output, &segs, &mut prog)?;
    } else {
        ui::stage(3, 3, "Encoding...");
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
        let mut prog = ui::Progress::new("Encoding", kept);
        transcode::run(&a.filename, &output, &det, &segs, &o, &mut prog)?;
    }
    ui::info(&format!("write step took {}", human(t.elapsed().as_secs_f64())));

    eprintln!();
    eprintln!("Done. Wrote {}", output.display());
    eprintln!("  cut (silent)  : {}  ({:.1}%)", hms(cut), pct(cut));
    eprintln!("  kept (sounded): {}  ({:.1}%)", hms(kept), pct(kept));
    eprintln!("  total elapsed : {}", ago(ui::elapsed_secs()));
    Ok(())
}
