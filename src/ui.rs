//! Human-readable output: stage headers, HH:MM:SS, "Xh Xm Xs ago", ETA and progress lines.

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;
use std::time::Instant;

static PROGRAM_START: OnceLock<Instant> = OnceLock::new();

/// Call once at program start; "elapsed" in all output is measured from here.
pub fn start_clock() {
    let _ = PROGRAM_START.set(Instant::now());
}

pub fn elapsed_secs() -> f64 {
    PROGRAM_START.get().map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0)
}

/// 3725.4 -> "01:02:05"
pub fn hms(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// 3725 -> "1h 2m 5s", 125 -> "2m 5s", 5 -> "5s" (sub-second: "0.04s").
pub fn human(secs: f64) -> String {
    let secs = secs.max(0.0);
    if secs < 1.0 {
        return format!("{secs:.2}s");
    }
    let s = secs.round() as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}h {m}m {sec}s")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}

/// Same as `human` with " ago" appended (used for elapsed time).
pub fn ago(secs: f64) -> String {
    format!("{} ago", human(secs))
}

pub fn stage(n: u32, of: u32, msg: &str) {
    eprintln!("[{n}/{of}] {msg}");
}

pub fn info(msg: &str) {
    eprintln!("      {msg}");
}

/// A throttled single-line progress display:
///   Encoding  00:00:05/00:00:12 (41.7%)  elapsed 3s ago  ETA 4s
pub struct Progress {
    label: &'static str,
    total: f64,
    began: Instant,
    last: Instant,
    tty: bool,
    drawn: bool,
}

impl Progress {
    pub fn new(label: &'static str, total_secs: f64) -> Self {
        let now = Instant::now();
        Self {
            label,
            total: total_secs,
            began: now,
            last: now - std::time::Duration::from_secs(60),
            tty: std::io::stderr().is_terminal(),
            drawn: false,
        }
    }

    pub fn set_total(&mut self, total_secs: f64) {
        self.total = total_secs;
    }

    fn line(&self, done: f64) -> String {
        let done = if self.total > 0.0 { done.clamp(0.0, self.total) } else { done.max(0.0) };
        let stage_elapsed = self.began.elapsed().as_secs_f64();
        if self.total > 0.0 {
            let frac = done / self.total;
            let eta = if frac >= 0.005 && frac < 1.0 {
                human(stage_elapsed * (1.0 - frac) / frac)
            } else if frac >= 1.0 {
                "0s".to_string()
            } else {
                "--".to_string()
            };
            format!(
                "  {:<10} {}/{} ({:5.1}%)  elapsed {}  ETA {}",
                self.label,
                hms(done),
                hms(self.total),
                frac * 100.0,
                ago(elapsed_secs()),
                eta
            )
        } else {
            format!("  {:<10} {}  elapsed {}", self.label, hms(done), ago(elapsed_secs()))
        }
    }

    /// `done` is how many seconds of media have been processed so far.
    pub fn update(&mut self, done: f64) {
        let gap = if self.tty { 0.2 } else { 5.0 };
        if self.last.elapsed().as_secs_f64() < gap {
            return;
        }
        self.last = Instant::now();
        let line = self.line(done);
        if self.tty {
            eprint!("\r{line}\x1b[K");
            let _ = std::io::stderr().flush();
            self.drawn = true;
        } else {
            eprintln!("{line}");
        }
    }

    /// Print the final 100% line and move to a fresh line.
    pub fn finish(&mut self) {
        let line = self.line(self.total.max(0.0));
        if self.tty && self.drawn {
            eprintln!("\r{line}\x1b[K");
        } else {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(hms(3725.0), "01:02:05");
        assert_eq!(hms(0.4), "00:00:00");
        assert_eq!(human(3725.0), "1h 2m 5s");
        assert_eq!(human(125.0), "2m 5s");
        assert_eq!(human(5.0), "5s");
        assert_eq!(human(3600.0), "1h 0m 0s");
        assert_eq!(human(0.04), "0.04s");
        assert_eq!(ago(65.0), "1m 5s ago");
    }
}
