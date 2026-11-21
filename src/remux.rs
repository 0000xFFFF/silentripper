//! Stream-copy path (`-c`): no decode, no encode. Packets are copied as-is; segment
//! starts are snapped back to the previous keyframe so the video starts cleanly.

use crate::segments::{locate, Segment};
use anyhow::{Context, Result};
use ffmpeg::{codec, encoder, format, media, Packet};
use ffmpeg_next as ffmpeg;
use std::path::Path;

pub fn run(input: &Path, output: &Path, snapped: &[Segment], prog: &mut crate::ui::Progress) -> Result<()> {
    prog.set_total(crate::segments::total(snapped));
    let mut ictx = format::input(input)?;
    let mut octx = format::output(output).with_context(|| format!("cannot create {}", output.display()))?;

    let mut map = vec![usize::MAX; ictx.nb_streams() as usize];
    let mut in_tb = vec![ffmpeg::Rational(0, 1); ictx.nb_streams() as usize];
    for kind in [media::Type::Video, media::Type::Audio] {
        if let Some(s) = ictx.streams().best(kind) {
            let mut o = octx.add_stream(encoder::find(codec::Id::None))?;
            o.set_parameters(s.parameters());
            unsafe { (*o.parameters().as_mut_ptr()).codec_tag = 0 };
            map[s.index()] = o.index();
            in_tb[s.index()] = s.time_base();
        }
    }
    octx.set_metadata(ictx.metadata().to_owned());
    octx.write_header()?;
    let out_tb: Vec<_> = (0..octx.nb_streams() as usize).map(|i| octx.stream(i).unwrap().time_base()).collect();

    let mut last_dts = vec![i64::MIN; ictx.nb_streams() as usize];
    let mut pkt = Packet::empty();
    loop {
        match pkt.read(&mut ictx) {
            Ok(()) => {
                let si = pkt.stream();
                let oi = map[si];
                if oi == usize::MAX {
                    continue;
                }
                let tb = in_tb[si];
                let f = f64::from(tb);
                let Some(ts) = pkt.dts().or(pkt.pts()) else { continue };
                let Some(i) = locate(snapped, ts as f64 * f) else { continue };
                let s = &snapped[i];
                prog.update((ts as f64 * f - s.start).max(0.0) + s.out_start);
                let shift = ((s.start - s.out_start) / f).round() as i64;
                pkt.set_pts(pkt.pts().map(|p| p - shift));
                pkt.set_dts(pkt.dts().map(|d| d - shift));
                pkt.set_stream(oi);
                pkt.rescale_ts(tb, out_tb[oi]);
                // Joins between clips can land two packets on the same dts; keep it strictly increasing.
                if let Some(d) = pkt.dts() {
                    let d = d.max(last_dts[oi].saturating_add(1));
                    last_dts[oi] = d;
                    pkt.set_dts(Some(d));
                    if pkt.pts().map_or(false, |p| p < d) {
                        pkt.set_pts(Some(d));
                    }
                }
                pkt.set_position(-1);
                pkt.write_interleaved(&mut octx)?;
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }
    octx.write_trailer()?;
    prog.finish();
    Ok(())
}
