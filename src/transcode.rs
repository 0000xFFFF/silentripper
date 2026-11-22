//! Re-encode path: decode -> keep only sounded segments -> encode, all inside libav.
//!
//! * Video decoder only runs inside "decode windows" (keyframe before a kept segment
//!   .. end of that segment); silent stretches are skipped at packet level.
//! * Audio is cut sample-accurately and re-chunked through an in-process FIFO.
//! * Video encoder: libx264/libx265, NVENC, VAAPI or AMF (see gpu.rs).

use crate::gpu::{self, Backend, Codec, VaapiCtx, Vendor};
use crate::segments::{self, locate, Segment};
use crate::silence::Detection;
use crate::ui::{self, add, Progress, Timings};
use anyhow::{anyhow, bail, Context, Result};
use ffmpeg::codec::{self, threading};
use ffmpeg::format::sample::Type as SampleType;
use ffmpeg::format::{self, Pixel, Sample};
use ffmpeg::software::{resampling, scaling};
use ffmpeg::util::channel_layout::ChannelLayout;
use ffmpeg::{decoder, encoder, frame, media, picture, Dictionary, Packet, Rational};
use ffmpeg_next as ffmpeg;
use std::path::Path;
use std::sync::mpsc::{channel, sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GpuMode {
    Off,
    Auto,
    Nvidia,
    Amd,
}

pub struct Opts {
    pub codec: Codec,
    pub gpu: GpuMode,
    pub crf: u32,
    pub preset: String,
    pub audio_bitrate: usize,
    /// Target video bitrate in kbit/s (switches from constant quality to bitrate mode).
    pub bitrate: Option<usize>,
    pub bframes: Option<i32>,
    pub gop: Option<u32>,
    /// Encoder threads (libx264/libx265).
    pub threads: Option<u32>,
    /// Video decoder threads (0 = auto).
    pub decode_threads: usize,
    /// Frames buffered between pipeline stages.
    pub queue: usize,
    pub async_depth: Option<u32>,
    pub vaapi_rc: Option<String>,
    pub vaapi_quality: Option<i32>,
    pub nvenc_preset: Option<String>,
    pub amf_quality: Option<String>,
    /// Raw `--enc-opt key=value` pairs, applied last (they win).
    pub enc_opts: Vec<(String, String)>,
    /// Stop after this many seconds of output (for quick benchmarks).
    pub limit: Option<f64>,
}

macro_rules! drain {
    ($enc:expr, $octx:expr, $idx:expr, $etb:expr) => {{
        let mut pkt = Packet::empty();
        while $enc.receive_packet(&mut pkt).is_ok() {
            let otb = $octx.stream($idx).unwrap().time_base();
            pkt.set_stream($idx);
            pkt.rescale_ts($etb, otb);
            pkt.write_interleaved($octx)?;
        }
    }};
}

// ───────────────────────────── video encoder ─────────────────────────────

struct VideoEnc {
    enc: encoder::Video,
    backend: Backend,
    vaapi: Option<Arc<VaapiCtx>>,
    sw_fmt: Pixel,
    tb: Rational,
    out_idx: usize,
    /// Human-readable list of the encoder options actually applied.
    desc: String,
}

fn setopt(d: &mut Dictionary, applied: &mut Vec<String>, k: &str, v: &str) {
    d.set(k, v);
    applied.push(format!("{k}={v}"));
}

fn open_video(
    backend: Backend,
    codec_kind: Codec,
    w: u32,
    h: u32,
    tb: Rational,
    fps: Option<Rational>,
    aspect: Rational,
    global_header: bool,
    o: &Opts,
) -> Result<(VideoEnc, ffmpeg::Codec)> {
    let name = backend.encoder_name(codec_kind);
    let c = encoder::find_by_name(name)
        .ok_or_else(|| anyhow!("encoder `{name}` is not available in this libav build"))?;
    let mut enc = codec::context::Context::new_with_codec(c).encoder().video()?;
    enc.set_width(w);
    enc.set_height(h);
    enc.set_time_base(tb);
    enc.set_frame_rate(fps);
    enc.set_aspect_ratio(aspect);
    if global_header {
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
    }

    let mut vaapi = None;
    let sw_fmt;
    if backend == Backend::Vaapi {
        let v = VaapiCtx::new(w, h)?;
        unsafe { v.attach(enc.as_mut_ptr()) };
        vaapi = Some(Arc::new(v));
        sw_fmt = Pixel::NV12;
    } else {
        enc.set_format(Pixel::YUV420P);
        sw_fmt = Pixel::YUV420P;
    }

    let q = o.crf.to_string();
    let mut d = Dictionary::new();
    let mut applied: Vec<String> = Vec::new();
    let kbps = o.bitrate;
    let mut user_keys: Vec<String> = Vec::new();
    match backend {
        Backend::Cpu => {
            match kbps {
                Some(_) => {}
                None => setopt(&mut d, &mut applied, "crf", &q),
            }
            setopt(&mut d, &mut applied, "preset", &o.preset);
            if codec_kind == Codec::Hevc {
                setopt(&mut d, &mut applied, "x265-params", "log-level=error");
            }
        }
        Backend::Nvenc => {
            setopt(&mut d, &mut applied, "preset", o.nvenc_preset.as_deref().unwrap_or("p4"));
            setopt(&mut d, &mut applied, "rc", "vbr");
            if kbps.is_none() {
                setopt(&mut d, &mut applied, "cq", &q);
                enc.set_bit_rate(0);
            }
        }
        Backend::Vaapi => {
            if kbps.is_none() {
                setopt(&mut d, &mut applied, "qp", &q);
            }
        }
        Backend::Amf => {
            if kbps.is_none() {
                setopt(&mut d, &mut applied, "rc", "cqp");
                setopt(&mut d, &mut applied, "qp_i", &q);
                setopt(&mut d, &mut applied, "qp_p", &q);
            } else {
                setopt(&mut d, &mut applied, "rc", "vbr_peak");
            }
            setopt(&mut d, &mut applied, "quality", o.amf_quality.as_deref().unwrap_or("speed"));
        }
    }
    if let Some(k) = kbps {
        enc.set_bit_rate(k * 1000);
        applied.push(format!("bit_rate={k}k"));
    }
    if let Some(n) = o.bframes {
        enc.set_max_b_frames(n.max(0) as usize);
        applied.push(format!("max_b_frames={n}"));
    }
    if let Some(g) = o.gop {
        enc.set_gop(g);
        applied.push(format!("gop={g}"));
    }
    if let Some(t) = o.threads {
        setopt(&mut d, &mut applied, "threads", &t.to_string());
        user_keys.push("threads".into());
    }
    if let Some(n) = o.async_depth {
        setopt(&mut d, &mut applied, "async_depth", &n.to_string());
        user_keys.push("async_depth".into());
    }
    if backend == Backend::Vaapi {
        if let Some(rc) = &o.vaapi_rc {
            setopt(&mut d, &mut applied, "rc_mode", rc);
        }
        if let Some(qv) = o.vaapi_quality {
            setopt(&mut d, &mut applied, "quality", &qv.to_string());
        }
    } else if o.vaapi_rc.is_some() || o.vaapi_quality.is_some() {
        bail!("--vaapi-rc / --vaapi-quality only apply to the VAAPI encoder (using {name})");
    }
    for (k, v) in &o.enc_opts {
        setopt(&mut d, &mut applied, k, v);
        user_keys.push(k.clone());
    }
    // libav silently ignores option names an encoder doesn't know; catch typos instead.
    for k in &user_keys {
        let ck = std::ffi::CString::new(k.as_str())?;
        let found = unsafe {
            ffmpeg::ffi::av_opt_find(
                enc.as_mut_ptr() as *mut std::ffi::c_void,
                ck.as_ptr(),
                std::ptr::null(),
                0,
                ffmpeg::ffi::AV_OPT_SEARCH_CHILDREN as i32,
            )
        };
        if found.is_null() {
            bail!("encoder `{name}` has no option `{k}` (see: silentripper --list-opts {name})");
        }
    }
    let desc = applied.join(" ");
    let opened = enc.open_with(d).with_context(|| format!("opening encoder `{name}` with options [{desc}]"))?;
    Ok((VideoEnc { enc: opened, backend, vaapi, sw_fmt, tb, out_idx: 0, desc }, c))
}

// ───────────────────────── encoder pipeline (threads) ─────────────────────────
//
//   main thread            convert thread              encode thread
//   demux + decode  ──▶    swscale + GPU upload  ──▶   send_frame/receive_packet
//   audio, muxing   ◀──────────────────── encoded packets ◀──────────────────────
//
// Each stage works on a different frame at the same time instead of strictly in turn.

struct VideoPipe {
    tx: Option<SyncSender<frame::Video>>,
    pkt_rx: Receiver<Packet>,
    handles: Vec<JoinHandle<Result<()>>>,
    tb: Rational,
    out_idx: usize,
    t: Arc<Timings>,
}

fn spawn_pipe(v: VideoEnc, t: Arc<Timings>, queue: usize) -> Result<VideoPipe> {
    let VideoEnc { enc, vaapi, sw_fmt, tb, out_idx, .. } = v;
    let queue = queue.max(1);
    let (tx, rx_in) = sync_channel::<frame::Video>(queue);
    let (tx_mid, rx_mid) = sync_channel::<frame::Video>(queue);
    let (pkt_tx, pkt_rx) = channel::<Packet>();

    let t1 = t.clone();
    let conv = std::thread::Builder::new().name("convert".into()).spawn(move || -> Result<()> {
        let mut scaler: Option<scaling::Context> = None; // created here: not Send
        for f in rx_in {
            let c0 = Instant::now();
            let out = if f.format() == sw_fmt {
                f
            } else {
                if scaler.is_none() {
                    scaler = Some(scaling::Context::get(
                        f.format(),
                        f.width(),
                        f.height(),
                        sw_fmt,
                        f.width(),
                        f.height(),
                        scaling::Flags::BILINEAR,
                    )?);
                }
                let mut conv = frame::Video::empty();
                scaler.as_mut().unwrap().run(&f, &mut conv)?;
                conv.set_pts(f.pts());
                conv
            };
            add(&t1.convert, c0.elapsed());
            let out = match &vaapi {
                Some(v) => {
                    let u0 = Instant::now();
                    let hw = v.upload(&out)?;
                    add(&t1.upload, u0.elapsed());
                    hw
                }
                None => out,
            };
            if tx_mid.send(out).is_err() {
                break;
            }
        }
        Ok(())
    })?;

    let t2 = t.clone();
    let encth = std::thread::Builder::new().name("encode".into()).spawn(move || -> Result<()> {
        let mut enc = enc;
        let drain = |enc: &mut encoder::Video| -> bool {
            loop {
                let mut pkt = Packet::empty();
                if enc.receive_packet(&mut pkt).is_err() {
                    return true;
                }
                if pkt_tx.send(pkt).is_err() {
                    return false;
                }
            }
        };
        for f in rx_mid {
            let e0 = Instant::now();
            enc.send_frame(&f)?;
            drop(f);
            let alive = drain(&mut enc);
            add(&t2.encode, e0.elapsed());
            if !alive {
                return Ok(());
            }
        }
        let e0 = Instant::now();
        enc.send_eof()?;
        drain(&mut enc);
        add(&t2.encode, e0.elapsed());
        Ok(())
    })?;

    Ok(VideoPipe { tx: Some(tx), pkt_rx, handles: vec![conv, encth], tb, out_idx, t })
}

impl VideoPipe {
    fn submit(&mut self, f: frame::Video) -> Result<()> {
        let s0 = Instant::now();
        let ok = self.tx.as_ref().map(|tx| tx.send(f).is_ok()).unwrap_or(false);
        add(&self.t.stall, s0.elapsed());
        if ok {
            Ok(())
        } else {
            Err(self.fail())
        }
    }

    /// A worker stopped early: find out why.
    fn fail(&mut self) -> anyhow::Error {
        self.tx = None;
        for h in self.handles.drain(..) {
            match h.join() {
                Ok(Err(e)) => return e,
                Err(_) => return anyhow!("video encoder thread panicked"),
                Ok(Ok(())) => {}
            }
        }
        anyhow!("video encoder pipeline stopped unexpectedly")
    }

    fn write(&self, mut pkt: Packet, octx: &mut format::context::Output) -> Result<()> {
        let m0 = Instant::now();
        let otb = octx.stream(self.out_idx).unwrap().time_base();
        pkt.set_stream(self.out_idx);
        pkt.rescale_ts(self.tb, otb);
        pkt.write_interleaved(octx)?;
        add(&self.t.mux, m0.elapsed());
        Ok(())
    }

    /// Write whatever packets the encoder has finished so far.
    fn pump(&mut self, octx: &mut format::context::Output) -> Result<()> {
        while let Ok(pkt) = self.pkt_rx.try_recv() {
            self.write(pkt, octx)?;
        }
        Ok(())
    }

    fn finish(&mut self, octx: &mut format::context::Output) -> Result<()> {
        self.tx = None; // closes the chain; workers flush and exit
        while let Ok(pkt) = self.pkt_rx.recv() {
            self.write(pkt, octx)?;
        }
        for h in self.handles.drain(..) {
            h.join().map_err(|_| anyhow!("video encoder thread panicked"))??;
        }
        Ok(())
    }
}

// ───────────────────────────── video path ─────────────────────────────

struct VideoPath<'a> {
    dec: decoder::Video,
    pipe: VideoPipe,
    t: Arc<Timings>,
    segs: &'a [Segment],
    windows: Vec<(f64, f64)>,
    tb: f64,
    last_pts: i64,
    last_out: f64,
    active: bool,
}

impl VideoPath<'_> {
    fn in_window(&self, t: f64) -> bool {
        let i = self.windows.partition_point(|w| w.1 <= t);
        i < self.windows.len() && self.windows[i].0 <= t
    }

    fn drain_decoded(&mut self, octx: &mut format::context::Output) -> Result<()> {
        loop {
            let mut fr = frame::Video::empty();
            let d0 = Instant::now();
            let r = self.dec.receive_frame(&mut fr);
            add(&self.t.decode, d0.elapsed());
            if r.is_err() {
                break;
            }
            let Some(ts) = fr.timestamp() else { continue };
            let t = ts as f64 * self.tb;
            // 2ms tolerance: a frame sitting exactly on a cut point must not be lost
            // to float noise in the detected silence boundary.
            if let Some(i) = locate(self.segs, t + 0.002) {
                let s = &self.segs[i];
                let out_t = (t - s.start).max(0.0) + s.out_start;
                let mut pts = (out_t / self.tb).round() as i64;
                if pts <= self.last_pts {
                    pts = self.last_pts + 1;
                }
                self.last_pts = pts;
                self.last_out = out_t;
                fr.set_pts(Some(pts));
                fr.set_kind(picture::Type::None);
                self.pipe.submit(fr)?;
                self.pipe.pump(octx)?;
            }
        }
        Ok(())
    }

    fn packet(&mut self, p: &Packet, octx: &mut format::context::Output) -> Result<()> {
        let t = p.dts().or(p.pts()).map(|x| x as f64 * self.tb).unwrap_or(0.0);
        if self.in_window(t) {
            self.active = true;
            let d0 = Instant::now();
            let _ = self.dec.send_packet(p);
            add(&self.t.decode, d0.elapsed());
            self.drain_decoded(octx)?;
        } else if self.active {
            self.cut(octx)?;
        }
        Ok(())
    }

    /// Leaving a decode window: drain the decoder and reset it for the next keyframe.
    fn cut(&mut self, octx: &mut format::context::Output) -> Result<()> {
        let _ = self.dec.send_eof();
        self.drain_decoded(octx)?;
        self.dec.flush();
        self.active = false;
        Ok(())
    }

    fn finish(&mut self, octx: &mut format::context::Output) -> Result<()> {
        if self.active {
            self.cut(octx)?;
        }
        self.pipe.finish(octx)
    }
}

// ───────────────────────────── audio path ─────────────────────────────

struct AudioPath {
    dec: decoder::Audio,
    swr: resampling::Context,
    enc: encoder::Audio,
    layout: ChannelLayout,
    channels: usize,
    rate: u32,
    fifo: Vec<Vec<f32>>,
    frame_size: usize,
    seg_samples: Vec<(u64, u64)>,
    cursor: usize,
    pos: u64,
    emitted: i64,
    out_idx: usize,
}

impl AudioPath {
    fn drain_decoded(&mut self, octx: &mut format::context::Output) -> Result<()> {
        let mut decoded = frame::Audio::empty();
        while self.dec.receive_frame(&mut decoded).is_ok() {
            decoded.set_channel_layout(self.layout);
            let mut conv = frame::Audio::empty();
            self.swr.run(&decoded, &mut conv)?;
            let n = conv.samples();
            if n == 0 {
                continue;
            }
            let (a, b) = (self.pos, self.pos + n as u64);
            self.pos = b;
            while self.cursor < self.seg_samples.len() && self.seg_samples[self.cursor].1 <= a {
                self.cursor += 1;
            }
            let mut c = self.cursor;
            while c < self.seg_samples.len() && self.seg_samples[c].0 < b {
                let lo = self.seg_samples[c].0.max(a);
                let hi = self.seg_samples[c].1.min(b);
                if hi > lo {
                    for ch in 0..self.channels {
                        let p = &conv.plane::<f32>(ch)[(lo - a) as usize..(hi - a) as usize];
                        self.fifo[ch].extend_from_slice(p);
                    }
                }
                c += 1;
            }
            self.pump(false, octx)?;
        }
        Ok(())
    }

    fn pump(&mut self, last: bool, octx: &mut format::context::Output) -> Result<()> {
        let fs = if self.frame_size > 0 { self.frame_size } else { 1024 };
        while self.fifo[0].len() >= fs || (last && !self.fifo[0].is_empty()) {
            let n = self.fifo[0].len().min(fs);
            let mut f = frame::Audio::new(Sample::F32(SampleType::Planar), n, self.layout);
            f.set_rate(self.rate);
            for ch in 0..self.channels {
                f.plane_mut::<f32>(ch)[..n].copy_from_slice(&self.fifo[ch][..n]);
                self.fifo[ch].drain(..n);
            }
            f.set_pts(Some(self.emitted));
            self.emitted += n as i64;
            self.enc.send_frame(&f)?;
            let (idx, tb) = (self.out_idx, Rational(1, self.rate as i32));
            drain!(self.enc, octx, idx, tb);
        }
        Ok(())
    }

    fn finish(&mut self, octx: &mut format::context::Output) -> Result<()> {
        let _ = self.dec.send_eof();
        self.drain_decoded(octx)?;
        self.pump(true, octx)?;
        self.enc.send_eof()?;
        let (idx, tb) = (self.out_idx, Rational(1, self.rate as i32));
        drain!(self.enc, octx, idx, tb);
        Ok(())
    }
}

// ───────────────────────────── driver ─────────────────────────────

fn backend_plan(o: &Opts) -> Vec<Backend> {
    match o.gpu {
        GpuMode::Off => {
            ui::info("GPU not requested, using the CPU encoder");
            vec![Backend::Cpu]
        }
        GpuMode::Nvidia => {
            ui::info("GPU: NVIDIA requested (--gpu-nvidia)");
            gpu::candidates(Vendor::Nvidia)
        }
        GpuMode::Amd => {
            ui::info("GPU: AMD requested (--gpu-amd)");
            gpu::candidates(Vendor::Amd)
        }
        GpuMode::Auto => {
            ui::info("Detecting GPU...");
            let vendors = gpu::detect_vendors();
            if vendors.is_empty() {
                ui::info("no NVIDIA/AMD GPU detected, using the CPU encoder");
            } else {
                let names: Vec<&str> = vendors
                    .iter()
                    .map(|v| if *v == Vendor::Nvidia { "NVIDIA" } else { "AMD" })
                    .collect();
                ui::info(&format!("detected GPU vendor(s): {}", names.join(", ")));
            }
            let mut v: Vec<Backend> = vendors.into_iter().flat_map(gpu::candidates).collect();
            v.push(Backend::Cpu);
            v
        }
    }
}

pub fn run(
    input: &Path,
    output: &Path,
    det: &Detection,
    segs: &[Segment],
    o: &Opts,
    prog: &mut Progress,
) -> Result<Backend> {
    let timings = Arc::new(Timings::default());
    let wall = Instant::now();
    let mut ictx = format::input(input)?;
    let mut octx = format::output(output)
        .with_context(|| format!("cannot create {}", output.display()))?;
    let global = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

    let vstream = ictx.streams().best(media::Type::Video);
    let astream = ictx.streams().best(media::Type::Audio).ok_or_else(|| anyhow!("no audio stream"))?;
    let a_in = astream.index();

    // ---- video ----
    let mut vpath: Option<VideoPath> = None;
    let mut v_in = usize::MAX;
    let mut chosen = Backend::Cpu;
    if let Some(vs) = vstream {
        v_in = vs.index();
        let vtb = vs.time_base();
        let mut dctx = codec::context::Context::from_parameters(vs.parameters())?;
        let mut cfg = threading::Config::default();
        cfg.kind = threading::Type::Frame;
        cfg.count = o.decode_threads;
        dctx.set_threading(cfg);
        let dec = dctx.decoder().video()?;

        let fr = vs.avg_frame_rate();
        let fps = if fr.numerator() > 0 && fr.denominator() > 0 { Some(fr) } else { None };
        let plan = backend_plan(o);
        let mut errors = Vec::new();
        let mut opened = None;
        for b in plan {
            ui::info(&format!("opening video encoder {}...", b.encoder_name(o.codec)));
            match open_video(b, o.codec, dec.width(), dec.height(), vtb, fps, dec.aspect_ratio(), global, o) {
                Ok(x) => {
                    opened = Some(x);
                    break;
                }
                Err(e) => errors.push(format!("{}: {e:#}", b.encoder_name(o.codec))),
            }
        }
        let (mut venc, vcodec) = match opened {
            Some(x) => x,
            None => bail!("no usable video encoder:\n  {}", errors.join("\n  ")),
        };
        for e in &errors {
            ui::info(&format!("unavailable: {e}"));
        }
        chosen = venc.backend;
        ui::info(&format!("encoder options: {}", if venc.desc.is_empty() { "(defaults)".into() } else { venc.desc.clone() }));
        ui::info(&format!(
            "using {} ({}), {}x{}",
            chosen.encoder_name(o.codec),
            match chosen {
                Backend::Cpu => "CPU",
                Backend::Nvenc => "GPU / NVIDIA NVENC",
                Backend::Vaapi => "GPU / AMD VAAPI",
                Backend::Amf => "GPU / AMD AMF",
            },
            dec.width(),
            dec.height()
        ));

        let mut ost = octx.add_stream(Some(vcodec))?;
        ost.set_parameters(&venc.enc);
        venc.out_idx = ost.index();

        let windows = if det.keyframes.is_empty() {
            vec![(f64::MIN, f64::MAX)]
        } else {
            segments::decode_windows(segs, &det.keyframes)
        };
        vpath = Some(VideoPath {
            dec,
            pipe: spawn_pipe(venc, timings.clone(), o.queue)?,
            t: timings.clone(),
            segs,
            windows,
            tb: f64::from(vtb),
            last_pts: -1,
            last_out: 0.0,
            active: false,
        });
    }

    // ---- audio ----
    let adec_ctx = codec::context::Context::from_parameters(astream.parameters())?;
    let adec = adec_ctx.decoder().audio()?;
    let rate = adec.rate();
    let layout = if adec.channel_layout().is_empty() {
        ChannelLayout::default(adec.channels() as i32)
    } else {
        adec.channel_layout()
    };
    let channels = layout.channels() as usize;
    let swr = resampling::Context::get(
        adec.format(),
        layout,
        rate,
        Sample::F32(SampleType::Planar),
        layout,
        rate,
    )?;
    let acodec = encoder::find(codec::Id::AAC).ok_or_else(|| anyhow!("AAC encoder missing"))?;
    let mut aenc = codec::context::Context::new_with_codec(acodec).encoder().audio()?;
    aenc.set_rate(rate as i32);
    aenc.set_channel_layout(layout);
    aenc.set_format(Sample::F32(SampleType::Planar));
    aenc.set_bit_rate(o.audio_bitrate);
    aenc.set_time_base(Rational(1, rate as i32));
    if global {
        aenc.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let aenc = aenc.open()?;
    let mut aost = octx.add_stream(Some(acodec))?;
    aost.set_parameters(&aenc);
    let a_out = aost.index();
    let frame_size = aenc.frame_size() as usize;

    let seg_samples: Vec<(u64, u64)> = segs
        .iter()
        .map(|s| {
            (
                ((s.start - det.start) * rate as f64).round().max(0.0) as u64,
                ((s.end - det.start) * rate as f64).round().max(0.0) as u64,
            )
        })
        .collect();
    let mut apath = AudioPath {
        dec: adec,
        swr,
        enc: aenc,
        layout,
        channels,
        rate,
        fifo: vec![Vec::new(); channels],
        frame_size,
        seg_samples,
        cursor: 0,
        pos: 0,
        emitted: 0,
        out_idx: a_out,
    };

    octx.set_metadata(ictx.metadata().to_owned());
    octx.write_header()?;

    // ---- main loop ----
    let kept_total = segments::total(segs);
    prog.set_total(o.limit.map_or(kept_total, |l| l.min(kept_total)));
    if let Some(l) = o.limit {
        ui::info(&format!("--limit: will stop after {} of output", ui::hms(l)));
    }
    let mut packet = Packet::empty();
    loop {
        match packet.read(&mut ictx) {
            Ok(()) => {
                let si = packet.stream();
                if si == a_in {
                    let a0 = Instant::now();
                    if apath.dec.send_packet(&packet).is_ok() {
                        apath.drain_decoded(&mut octx)?;
                    }
                    add(&timings.audio, a0.elapsed());
                    if vpath.is_none() {
                        prog.update(apath.emitted as f64 / apath.rate as f64);
                    }
                } else if si == v_in {
                    if let Some(v) = vpath.as_mut() {
                        v.packet(&packet, &mut octx)?;
                        prog.update(v.last_out);
                    }
                }
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(e.into()),
        }
        if let Some(l) = o.limit {
            let cur = match vpath.as_ref() {
                Some(v) => v.last_out,
                None => apath.emitted as f64 / apath.rate as f64,
            };
            if cur >= l {
                break;
            }
        }
    }
    ui::info("flushing encoders and writing trailer...");

    if let Some(v) = vpath.as_mut() {
        v.finish(&mut octx)?;
    }
    apath.finish(&mut octx)?;
    octx.write_trailer()?;
    prog.finish();

    let t = &timings;
    ui::info(&format!(
        "stage busy time (stages overlap; wall clock {}):",
        ui::human(wall.elapsed().as_secs_f64())
    ));
    ui::info(&format!("  video decode      : {}", ui::human(ui::secs(&t.decode))));
    ui::info(&format!("  colour convert    : {}", ui::human(ui::secs(&t.convert))));
    if chosen == Backend::Vaapi {
        ui::info(&format!("  GPU upload        : {}", ui::human(ui::secs(&t.upload))));
    }
    ui::info(&format!("  video encode      : {}  ({})", ui::human(ui::secs(&t.encode)), chosen.encoder_name(o.codec)));
    ui::info(&format!("  audio             : {}", ui::human(ui::secs(&t.audio))));
    ui::info(&format!("  muxing            : {}", ui::human(ui::secs(&t.mux))));
    ui::info(&format!("  decoder waiting on encoder: {}", ui::human(ui::secs(&t.stall))));
    Ok(chosen)
}
