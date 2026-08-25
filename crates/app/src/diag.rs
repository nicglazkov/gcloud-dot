//! A small diagnostic log for the tray.
//!
//! The tray is a GUI process: it has no console, so everything written to
//! stderr vanishes. When the app misbehaved on a machine six days into an
//! uptime, the only evidence was what the operating system happened to keep,
//! and reconstructing events from Defender timestamps and file mtimes took an
//! evening that a log would have made a five minute read.
//!
//! Plain lines in a plain file, beside the state. Not a logging framework:
//! the tray notes a couple of dozen events per day, and the reader is a
//! person with a problem.

use std::io::Write;
use std::path::PathBuf;

/// Where the log lives, beside the state file.
pub fn path() -> PathBuf {
    gcloud_dot_core::paths::state_path()
        .parent()
        .map(|d| d.join("tray.log"))
        .unwrap_or_else(|| PathBuf::from("tray.log"))
}

/// Keep the log from growing without bound.
///
/// Checked on write rather than on a schedule. Half is kept, so a heal loop
/// that writes every five seconds cannot also fill the disk.
const CAP_BYTES: u64 = 256 * 1024;

/// Append one timestamped line. Failures are swallowed: a diagnostic that can
/// take the app down is worse than no diagnostic.
pub fn note(msg: &str) {
    let p = path();
    if let Ok(meta) = std::fs::metadata(&p) {
        if meta.len() > CAP_BYTES {
            trim(&p);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(
            f,
            "{}  {msg}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }
}

/// Drop the older half, keeping whole lines.
fn trim(p: &std::path::Path) {
    let Ok(s) = std::fs::read_to_string(p) else {
        return;
    };
    let half = s.len() / 2;
    let tail = match s[half..].find('\n') {
        Some(i) => &s[half + i + 1..],
        None => "",
    };
    let _ = std::fs::write(p, tail);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_keeps_the_recent_half_in_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tray.log");
        let lines: Vec<String> = (0..100).map(|i| format!("line number {i}")).collect();
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        trim(&p);
        let kept = std::fs::read_to_string(&p).unwrap();
        assert!(
            kept.starts_with("line number"),
            "must start on a line boundary: {kept:.20}"
        );
        assert!(kept.contains("line number 99"), "the newest line survives");
        assert!(!kept.contains("line number 0\n"), "the oldest does not");
    }
}
