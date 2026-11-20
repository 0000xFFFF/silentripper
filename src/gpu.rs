//! GPU detection and hardware encoder selection (all through libavcodec).
//!
//! NVIDIA -> `*_nvenc`
//! AMD    -> `*_vaapi` on Linux (Mesa), `*_amf` on Windows / AMD-PRO
//!
//! `--gpu` detects the vendor from the PCI ids (Linux sysfs) and falls back to probing
//! which encoders actually open. Explicit `--gpu-nvidia` / `--gpu-amd` never fall back
//! silently: if the encoder can't be opened you get the real libav error.

use anyhow::{anyhow, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::ffi;
use std::ffi::CString;
use std::ptr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Nvenc,
    Vaapi,
    Amf,
}

impl Backend {
    pub fn encoder_name(self, c: Codec) -> &'static str {
        match (self, c) {
            (Backend::Cpu, Codec::H264) => "libx264",
            (Backend::Cpu, Codec::Hevc) => "libx265",
            (Backend::Nvenc, Codec::H264) => "h264_nvenc",
            (Backend::Nvenc, Codec::Hevc) => "hevc_nvenc",
            (Backend::Vaapi, Codec::H264) => "h264_vaapi",
            (Backend::Vaapi, Codec::Hevc) => "hevc_vaapi",
            (Backend::Amf, Codec::H264) => "h264_amf",
            (Backend::Amf, Codec::Hevc) => "hevc_amf",
        }
    }
}

/// Backends to try (in order) for a vendor.
pub fn candidates(v: Vendor) -> Vec<Backend> {
    match v {
        Vendor::Nvidia => vec![Backend::Nvenc],
        Vendor::Amd => {
            if cfg!(windows) {
                vec![Backend::Amf]
            } else {
                vec![Backend::Vaapi, Backend::Amf]
            }
        }
    }
}

/// Every GPU vendor we can see, best first.
pub fn detect_vendors() -> Vec<Vendor> {
    let mut found = Vec::new();
    #[cfg(target_os = "linux")]
    {
        if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
            let mut cards: Vec<_> = rd.flatten().collect();
            cards.sort_by_key(|e| e.file_name());
            for e in cards {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with("card") || name.contains('-') {
                    continue;
                }
                if let Ok(v) = std::fs::read_to_string(e.path().join("device/vendor")) {
                    match v.trim() {
                        "0x10de" => found.push(Vendor::Nvidia),
                        "0x1002" => found.push(Vendor::Amd),
                        _ => {}
                    }
                }
            }
        }
        if found.is_empty() && std::path::Path::new("/proc/driver/nvidia").exists() {
            found.push(Vendor::Nvidia);
        }
    }
    // Non-Linux (or nothing in sysfs): probe by encoder presence. The real open test
    // happens when the encoder is created.
    if found.is_empty() {
        if ffmpeg::encoder::find_by_name("h264_nvenc").is_some() {
            found.push(Vendor::Nvidia);
        }
        if ffmpeg::encoder::find_by_name("h264_amf").is_some() {
            found.push(Vendor::Amd);
        }
    }
    // Prefer a discrete NVIDIA card over an AMD iGPU when both exist.
    found.sort_by_key(|v| if *v == Vendor::Nvidia { 0 } else { 1 });
    found.dedup();
    found
}

/// Linux DRM render node belonging to an AMD GPU (for VAAPI).
pub fn amd_render_node() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let mut nodes: Vec<_> = std::fs::read_dir("/sys/class/drm").ok()?.flatten().collect();
        nodes.sort_by_key(|e| e.file_name());
        for e in nodes {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("renderD") {
                if let Ok(v) = std::fs::read_to_string(e.path().join("device/vendor")) {
                    if v.trim() == "0x1002" {
                        return Some(format!("/dev/dri/{name}"));
                    }
                }
            }
        }
    }
    None
}

/// An initialised VAAPI device + frame pool (frames get uploaded to it before encoding).
pub struct VaapiCtx {
    frames: *mut ffi::AVBufferRef,
    device: *mut ffi::AVBufferRef,
}

impl VaapiCtx {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        unsafe {
            let node = amd_render_node().unwrap_or_else(|| "/dev/dri/renderD128".into());
            let cnode = CString::new(node.clone())?;
            let mut dev: *mut ffi::AVBufferRef = ptr::null_mut();
            let r = ffi::av_hwdevice_ctx_create(
                &mut dev,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                cnode.as_ptr(),
                ptr::null_mut(),
                0,
            );
            if r < 0 {
                return Err(anyhow!("cannot open VAAPI device {node}: {}", ffmpeg::Error::from(r)));
            }
            let frames = ffi::av_hwframe_ctx_alloc(dev);
            if frames.is_null() {
                ffi::av_buffer_unref(&mut dev);
                return Err(anyhow!("av_hwframe_ctx_alloc failed"));
            }
            let fc = (*frames).data as *mut ffi::AVHWFramesContext;
            (*fc).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*fc).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*fc).width = width as i32;
            (*fc).height = height as i32;
            (*fc).initial_pool_size = 24;
            let r = ffi::av_hwframe_ctx_init(frames);
            if r < 0 {
                let mut f = frames;
                ffi::av_buffer_unref(&mut f);
                ffi::av_buffer_unref(&mut dev);
                return Err(anyhow!("VAAPI frame pool init failed: {}", ffmpeg::Error::from(r)));
            }
            Ok(Self { frames, device: dev })
        }
    }

    /// Attach the frame pool to an (unopened) encoder context.
    pub unsafe fn attach(&self, ctx: *mut ffi::AVCodecContext) {
        (*ctx).hw_frames_ctx = ffi::av_buffer_ref(self.frames);
        (*ctx).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
    }

    /// Upload a software NV12 frame to the GPU. pts/props are copied across.
    pub fn upload(&self, sw: &ffmpeg::frame::Video) -> Result<ffmpeg::frame::Video> {
        unsafe {
            let mut hw = ffmpeg::frame::Video::empty();
            let r = ffi::av_hwframe_get_buffer(self.frames, hw.as_mut_ptr(), 0);
            if r < 0 {
                return Err(anyhow!("av_hwframe_get_buffer: {}", ffmpeg::Error::from(r)));
            }
            let r = ffi::av_hwframe_transfer_data(hw.as_mut_ptr(), sw.as_ptr(), 0);
            if r < 0 {
                return Err(anyhow!("av_hwframe_transfer_data: {}", ffmpeg::Error::from(r)));
            }
            ffi::av_frame_copy_props(hw.as_mut_ptr(), sw.as_ptr());
            Ok(hw)
        }
    }
}

impl Drop for VaapiCtx {
    fn drop(&mut self) {
        unsafe {
            ffi::av_buffer_unref(&mut self.frames);
            ffi::av_buffer_unref(&mut self.device);
        }
    }
}
