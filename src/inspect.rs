//! `--list-encoders` / `--list-opts`: show which encoders this libav build has and
//! which private options each accepts (everything you can pass with `--enc-opt k=v`).

use anyhow::{bail, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::ffi;
use std::ffi::{c_void, CStr, CString};
use std::ptr;

const KNOWN: &[(&str, &str)] = &[
    ("libx264", "CPU  H.264"),
    ("libx265", "CPU  HEVC"),
    ("h264_nvenc", "NVIDIA H.264"),
    ("hevc_nvenc", "NVIDIA HEVC"),
    ("h264_vaapi", "AMD/Intel (Linux) H.264"),
    ("hevc_vaapi", "AMD/Intel (Linux) HEVC"),
    ("h264_amf", "AMD (Windows) H.264"),
    ("hevc_amf", "AMD (Windows) HEVC"),
];

pub fn list_encoders() {
    println!("Video encoders silentripper can use (compiled into this libav):");
    for (name, what) in KNOWN {
        let have = ffmpeg::encoder::find_by_name(name).is_some();
        println!("  {:<12} {:<26} {}", name, what, if have { "available" } else { "-" });
    }
    println!("\n'available' only means libav was built with it; it still needs the driver/GPU at run time.");
    println!("Show its options:  silentripper --list-opts h264_vaapi");
}

unsafe fn cstr(p: *const std::os::raw::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

pub fn list_options(name: &str) -> Result<()> {
    let cname = CString::new(name)?;
    unsafe {
        let c = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
        if c.is_null() {
            bail!("encoder `{name}` is not in this libav build (see --list-encoders)");
        }
        println!("{name} - {}", cstr((*c).long_name));
        let pc = (*c).priv_class;
        if pc.is_null() {
            println!("  (no private options)");
        } else {
            println!("Private options, usable as:  --enc-opt NAME=VALUE\n");
            let obj: *const *const ffi::AVClass = &pc;
            let mut prev: *const ffi::AVOption = ptr::null();
            loop {
                prev = ffi::av_opt_next(obj as *const c_void, prev);
                if prev.is_null() {
                    break;
                }
                let o = &*prev;
                let oname = cstr(o.name);
                let help = cstr(o.help);
                let ty = format!("{:?}", o.type_).replace("AV_OPT_TYPE_", "").to_lowercase();
                if ty == "const" {
                    println!("        = {:<22} ({}) {}", oname, o.default_val.i64_, help);
                    continue;
                }
                let default = match ty.as_str() {
                    "float" | "double" => format!("{}", o.default_val.dbl),
                    "string" => cstr(o.default_val.str_),
                    "binary" | "dict" | "image_size" | "video_rate" | "color" | "rational" => String::new(),
                    _ => format!("{}", o.default_val.i64_),
                };
                println!("  {:<24} <{}> default: {}\n      {}", oname, ty, if default.is_empty() { "-" } else { &default }, help);
            }
        }
    }
    println!("\nGeneric libav codec options also work with --enc-opt, e.g. threads=8, bf=0, g=120, maxrate=10M.");
    Ok(())
}
