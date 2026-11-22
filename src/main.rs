mod gpu;
mod inspect;
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

#[derive(Copy, Clone, ValueEnum)]
enum LogLevel {
    Quiet,
    Error,
    Warning,
    Info,
    Verbose,
    Debug,
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err("expected KEY=VALUE".into()),
    }
}

/// Remove silent parts from video. Pure libav: no ffmpeg/ffprobe subprocesses.
#[derive(Parser)]
#[command(version, about)]
#[command(group(ArgGroup::new("gpumode").args(["gpu", "gpu_nvidia", "gpu_amd"])))]
struct Args {
    /// Input video file
    #[arg(required_unless_present_any = ["list_encoders", "list_opts"])]
    filename: Option<PathBuf>,

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

    // ───────────── experiment / tuning options ─────────────
    /// Video bitrate in kbit/s (bitrate mode instead of constant quality; ignores --crf)
    #[arg(long, value_name = "kbps", help_heading = "Encoder tuning")]
    bitrate: Option<usize>,

    /// Max consecutive B-frames (0 = none; GPU encoders are often faster with 0)
    #[arg(long, value_name = "N", help_heading = "Encoder tuning")]
    bframes: Option<i32>,

    /// Keyframe interval in frames
    #[arg(long, value_name = "frames", help_heading = "Encoder tuning")]
    gop: Option<u32>,

    /// Encoder threads (libx264/libx265)
    #[arg(long, value_name = "N", help_heading = "Encoder tuning")]
    threads: Option<u32>,

    /// Video decoder threads (0 = auto)
    #[arg(long, value_name = "N", default_value_t = 0, help_heading = "Encoder tuning")]
    decode_threads: usize,

    /// Frames buffered between decode / convert / encode threads
    #[arg(long, value_name = "N", default_value_t = 6, help_heading = "Encoder tuning")]
    queue: usize,

    /// VAAPI: frames in flight inside the encoder (libav default 2, max 64)
    #[arg(long, value_name = "N", help_heading = "Encoder tuning")]
    async_depth: Option<u32>,

    /// VAAPI rate control: auto, CQP, CBR, VBR, ICQ, QVBR, AVBR
    #[arg(long, value_name = "MODE", help_heading = "Encoder tuning")]
    vaapi_rc: Option<String>,

    /// VAAPI quality/speed trade-off (higher = faster, driver-dependent range)
    #[arg(long, value_name = "N", allow_negative_numbers = true, help_heading = "Encoder tuning")]
    vaapi_quality: Option<i32>,

    /// NVENC preset: p1 (fastest) .. p7 (best quality)
    #[arg(long, value_name = "p1..p7", help_heading = "Encoder tuning")]
    nvenc_preset: Option<String>,

    /// AMF quality: speed, balanced, quality
    #[arg(long, value_name = "MODE", help_heading = "Encoder tuning")]
    amf_quality: Option<String>,

    /// Pass any libav encoder option straight through (repeatable, applied last)
    #[arg(long = "enc-opt", value_name = "KEY=VALUE", value_parser = parse_kv, help_heading = "Encoder tuning")]
    enc_opt: Vec<(String, String)>,

    // ───────────── diagnostics ─────────────
    /// Only encode this many seconds of output, then finish (quick benchmarks)
    #[arg(long, value_name = "sec", help_heading = "Diagnostics")]
    limit: Option<f64>,

    /// Detect silence and print the plan, but write nothing
    #[arg(long, help_heading = "Diagnostics")]
    dry_run: bool,

    /// libav log level (default: error)
    #[arg(long, value_enum, value_name = "LEVEL", help_heading = "Diagnostics")]
    log_level: Option<LogLevel>,

    /// List the video encoders available in this libav build and exit
    #[arg(long, help_heading = "Diagnostics")]
    list_encoders: bool,

    /// List the options of an encoder (e.g. h264_vaapi) and exit
    #[arg(long, value_name = "ENCODER", help_heading = "Diagnostics")]
    list_opts: Option<String>,
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
    ffmpeg::log::set_level(match a.log_level {
        Some(LogLevel::Quiet) => ffmpeg::log::Level::Quiet,
        Some(LogLevel::Error) => ffmpeg::log::Level::Error,
        Some(LogLevel::Warning) => ffmpeg::log::Level::Warning,
        Some(LogLevel::Info) => ffmpeg::log::Level::Info,
        Some(LogLevel::Verbose) => ffmpeg::log::Level::Verbose,
        Some(LogLevel::Debug) => ffmpeg::log::Level::Debug,
        None if std::env::var_os("SILENTRIPPER_LOG").is_some() => ffmpeg::log::Level::Info,
        None => ffmpeg::log::Level::Error,
    });

    if a.list_encoders {
        inspect::list_encoders();
        return Ok(());
    }
    if let Some(name) = &a.list_opts {
        return inspect::list_options(name);
    }
    let input: PathBuf = a.filename.clone().expect("clap guarantees the input file");

    if !input.exists() {
        bail!("{} does not exist", input.display());
    }
    let output = a.output.clone().unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "mp4".into());
        input.with_file_name(format!("{stem}_ripped.{ext}"))
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
    eprintln!("  input : {}", input.display());
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
    let det = silence::detect(&input, a.noise, a.min_duration_silence, &mut prog)?;
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

    if a.dry_run {
        eprintln!("Dry run: nothing written (elapsed {}).", ago(ui::elapsed_secs()));
        return Ok(());
    }

    // [3/3] write
    pause(a.pause, "write output?");
    let t = Instant::now();
    if a.copy {
        ui::stage(3, 3, "Cutting and copying streams (no re-encode)...");
        let mut prog = ui::Progress::new("Copying", kept);
        remux::run(&input, &output, &segs, &mut prog)?;
    } else {
        ui::stage(3, 3, "Encoding...");
        let o = Opts {
            bitrate: a.bitrate,
            bframes: a.bframes,
            gop: a.gop,
            threads: a.threads,
            decode_threads: a.decode_threads,
            queue: a.queue,
            async_depth: a.async_depth,
            vaapi_rc: a.vaapi_rc.clone(),
            vaapi_quality: a.vaapi_quality,
            nvenc_preset: a.nvenc_preset.clone(),
            amf_quality: a.amf_quality.clone(),
            enc_opts: a.enc_opt.clone(),
            limit: a.limit,
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
        transcode::run(&input, &output, &det, &segs, &o, &mut prog)?;
    }
    ui::info(&format!("write step took {}", human(t.elapsed().as_secs_f64())));

    eprintln!();
    eprintln!("Done. Wrote {}", output.display());
    eprintln!("  cut (silent)  : {}  ({:.1}%)", hms(cut), pct(cut));
    eprintln!("  kept (sounded): {}  ({:.1}%)", hms(kept), pct(kept));
    eprintln!("  total elapsed : {}", ago(ui::elapsed_secs()));
    Ok(())
}
