//! Things the user asks for from the menu or the panel.
//!
//! Signing in runs `gcloud auth login` directly, with no terminal window at
//! all. gcloud opens your browser itself, which is the whole of the visible
//! flow; a terminal alongside it only added a window to close afterwards.
//!
//! Closing that window turned out to be impossible to do honestly. Terminal
//! refuses to close a window whose process is still running, and it refuses
//! quietly: the AppleScript returns success and the window stays. Asking
//! Terminal from outside is worse, because macOS then wants an automation
//! grant this app otherwise never needs.
//!
//! So the output is captured instead. On success nothing appears, because the
//! dot going green is the answer. On failure the reason is read out of the
//! captured output and shown as a notification, which is the only case where
//! any of it was worth reading.

use std::path::Path;
use std::process::Stdio;

/// Start `gcloud auth login`, detached, with its output captured.
///
/// Returns the child so the caller can wait on it without blocking the event
/// loop. gcloud launches the browser itself, so there is nothing for a user to
/// watch in the meantime.
pub fn login(gcloud: &Path) -> std::io::Result<std::process::Child> {
    let log = login_log_path();
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let out = std::fs::File::create(&log)?;
    let err = out.try_clone()?;

    let mut cmd = gcloud_dot_core::proc::quiet(gcloud);
    cmd.args(["auth", "login"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));

    cmd.spawn()
}

/// Where the captured output of the last sign in attempt is kept.
pub fn login_log_path() -> std::path::PathBuf {
    gcloud_dot_core::paths::data_dir().join("last-signin.log")
}

/// The line worth showing a person when a sign in fails.
///
/// gcloud prints a banner, a stack of URLs, and then the sentence that matters,
/// so the last non-empty line that is not a bare marker is a better answer than
/// the first.
pub fn failure_reason(output: &str) -> String {
    let line = output
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.eq_ignore_ascii_case("error:"))
        .unwrap_or("");
    let cleaned = line.trim_start_matches("ERROR:").trim();
    if cleaned.is_empty() {
        return "gcloud did not say why.".to_string();
    }
    let mut out: String = cleaned.chars().take(180).collect();
    if cleaned.chars().count() > 180 {
        out.push_str(" (truncated)");
    }
    out
}

/// Switch the active gcloud configuration.
pub fn activate_config(gcloud: &Path, name: &str) -> std::io::Result<()> {
    let status = gcloud_dot_core::proc::quiet(gcloud)
        .args(["config", "configurations", "activate", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "gcloud refused to activate {name}"
        )))
    }
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = gcloud_dot_core::proc::quiet("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = gcloud_dot_core::proc::quiet("cmd.exe");
        c.args(["/c", "start", ""]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = gcloud_dot_core::proc::quiet("xdg-open");

    cmd.arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_failure_reason_is_the_sentence_that_matters() {
        // gcloud prints the banner and the URL first, and the explanation last,
        // so the last meaningful line is the one a person needs.
        let out = "Your browser has been opened to visit:\n\n                       https://accounts.google.com/o/oauth2/auth?x=1\n\n                   ERROR: (gcloud.auth.login) The browser window was closed.";
        assert_eq!(
            super::failure_reason(out),
            "(gcloud.auth.login) The browser window was closed."
        );
    }

    #[test]
    fn a_silent_failure_still_says_something() {
        assert_eq!(super::failure_reason("   \n\n"), "gcloud did not say why.");
        assert_eq!(super::failure_reason(""), "gcloud did not say why.");
    }

    #[test]
    fn a_failure_reason_is_bounded() {
        let out = format!("ERROR: {}", "x".repeat(400));
        let r = super::failure_reason(&out);
        assert!(r.chars().count() <= 192);
        assert!(r.ends_with(" (truncated)"));
    }
}
