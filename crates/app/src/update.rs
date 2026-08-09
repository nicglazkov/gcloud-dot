//! Checking whether a newer release exists.
//!
//! Checks and reports. It never replaces a binary: on every platform the app
//! is installed by something that owns its files, Homebrew, winget, apt, an
//! installer, and a self-updater writing over those leaves the package
//! manager describing a version that is no longer on disk.
//!
//! The request is made with `curl`, which ships with macOS, Windows 10 1803 and
//! later, and effectively every Linux. Linking a TLS stack for one request a
//! day would add more to the binary than the rest of the app puts together.

use std::process::Stdio;
use std::time::Duration;

const RELEASES_API: &str = "https://api.github.com/repos/nicglazkov/gcloud-dot/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/nicglazkov/gcloud-dot/releases/latest";

/// Returns the newer version's tag, or `None` if this build is current or the
/// check could not be made.
pub fn check() -> Option<String> {
    let out = gcloud_dot_core::proc::quiet("curl")
        .args([
            "-fsSL",
            "--max-time",
            "15",
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            concat!("gcloud-dot/", env!("CARGO_PKG_VERSION")),
            RELEASES_API,
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = body.get("tag_name")?.as_str()?.trim_start_matches('v');
    is_newer(tag, gcloud_dot_core::VERSION).then(|| tag.to_string())
}

/// Spawn the check on a worker thread and hand the result back.
pub fn check_in_background<F: FnOnce(String) + Send + 'static>(delay: Duration, on_found: F) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if let Some(version) = check() {
            on_found(version);
        }
    });
}

/// Numeric comparison of dotted versions.
///
/// Anything non-numeric compares as zero, which makes a pre-release tag sort
/// below the release it precedes, the safe direction, since the alternative
/// is nagging every user of a stable build to "upgrade" to a beta.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split(['.', '-', '+'])
            .map(|p| p.parse().unwrap_or(0))
            .take(4)
            .collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_newer_versions() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn ignores_the_same_or_older() {
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.9", "1.0.0"));
        assert!(!is_newer("1.0", "1.0.0"));
    }

    #[test]
    fn a_prerelease_never_prompts_a_stable_user() {
        assert!(!is_newer("1.0.0-rc1", "1.0.0"));
        assert!(is_newer("1.0.1-rc1", "1.0.0"));
    }

    #[test]
    fn nonsense_tags_do_not_prompt_an_upgrade() {
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
    }
}
