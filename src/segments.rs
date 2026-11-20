//! Turn silence intervals into the list of sounded segments to keep.

use crate::silence::Detection;

#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    /// Where this segment starts on the output timeline (seconds).
    pub out_start: f64,
}

impl Segment {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }
}

/// Invert the silences into sounded segments, pad them, merge overlaps and drop
/// segments shorter than `min_sound`.
pub fn sounded(det: &Detection, min_sound: f64, pad: f64) -> Vec<Segment> {
    let mut raw: Vec<(f64, f64)> = Vec::with_capacity(det.silences.len() + 1);
    let mut cursor = det.start;
    for s in &det.silences {
        if s.start > cursor {
            raw.push((cursor, s.start));
        }
        cursor = cursor.max(s.end);
    }
    if det.end > cursor {
        raw.push((cursor, det.end));
    }

    // Pad + merge.
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(raw.len());
    for (a, b) in raw {
        let a = (a - pad).max(det.start);
        let b = (b + pad).min(det.end);
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }

    let mut out = Vec::with_capacity(merged.len());
    let mut acc = 0.0;
    for (a, b) in merged {
        let len = b - a;
        if len < min_sound.max(0.001) {
            continue;
        }
        out.push(Segment { start: a, end: b, out_start: acc });
        acc += len;
    }
    out
}

pub fn total(segs: &[Segment]) -> f64 {
    segs.iter().map(Segment::len).sum()
}

/// Index of the segment containing `t`, if any.
#[inline]
pub fn locate(segs: &[Segment], t: f64) -> Option<usize> {
    let i = segs.partition_point(|s| s.end <= t);
    (i < segs.len() && segs[i].start <= t).then_some(i)
}

/// For stream-copy: move each start back to the preceding keyframe so video decodes
/// cleanly, merge overlaps and recompute the output timeline.
pub fn snap_to_keyframes(segs: &[Segment], keys: &[f64]) -> Vec<Segment> {
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(segs.len());
    for s in segs {
        let i = keys.partition_point(|&k| k <= s.start + 1e-6);
        let a = if i > 0 { keys[i - 1] } else { s.start };
        match merged.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(s.end),
            _ => merged.push((a, s.end)),
        }
    }
    let mut acc = 0.0;
    merged
        .into_iter()
        .map(|(a, b)| {
            let seg = Segment { start: a, end: b, out_start: acc };
            acc += b - a;
            seg
        })
        .collect()
}

/// Windows in which the video decoder must run: from the keyframe preceding each
/// segment start to the segment end. Everything outside is skipped without decoding.
pub fn decode_windows(segs: &[Segment], keys: &[f64]) -> Vec<(f64, f64)> {
    snap_to_keyframes(segs, keys).into_iter().map(|s| (s.start, s.end)).collect()
}
