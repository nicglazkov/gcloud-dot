//! Finding the gcloud CLI.
//!
//! This is fiddlier than it looks. A tray app launched by launchd, a scheduled
//! task, or a desktop file inherits almost no `PATH`, so the interactive shell
//! trick that works in a terminal finds nothing. Known install locations are
//! checked first for that reason, and the shell is only a fallback.

use std::path::{Path, PathBuf};

#[cfg(not(windows))]
const CANDIDATES: &[&str] = &[
    "/opt/homebrew/bin/gcloud",
    "/usr/local/bin/gcloud",
    "/opt/homebrew/share/google-cloud-sdk/bin/gcloud",
    "/usr/local/share/google-cloud-sdk/bin/gcloud",
    "/usr/lib/google-cloud-sdk/bin/gcloud",
    "/usr/share/google-cloud-sdk/bin/gcloud",
    "/snap/bin/gcloud",
    "/usr/bin/gcloud",
];

/// Locations under the home directory worth checking.
///
/// Deliberately none of Downloads, Desktop, or Documents. macOS gates those
/// behind a consent prompt, so merely asking whether a file exists inside one
/// makes the system ask the user for access. This app is otherwise free of
/// permission prompts, and "GCloud Dot wants to access your Downloads folder"
/// is an alarming thing for a credential monitor to say.
///
/// Downloads was on this list, because it is where the SDK tarball lands. It is
/// where the tarball is extracted before being moved somewhere permanent, not
/// where anybody runs it from, so the prompt bought nothing.
#[cfg(not(windows))]
const HOME_RELATIVE: &[&str] = &[
    "google-cloud-sdk/bin/gcloud",
    ".local/share/google-cloud-sdk/bin/gcloud",
];

/// Directories macOS will not let a program look inside without asking.
///
/// Only the test reads this. It exists so the rule is written down next to the
/// list it constrains, rather than living in a reviewer's memory.
#[cfg(test)]
#[cfg(not(windows))]
const CONSENT_GATED: &[&str] = &[
    "Downloads",
    "Desktop",
    "Documents",
    "Library/Mobile Documents",
];

/// Locate the gcloud executable, or `None` if it is not installed.
pub fn find() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GCLOUD_DOT_GCLOUD") {
        let p = PathBuf::from(explicit);
        if is_executable(&p) {
            return Some(p);
        }
    }

    #[cfg(windows)]
    {
        find_windows()
    }
    #[cfg(not(windows))]
    {
        for c in CANDIDATES {
            let p = PathBuf::from(c);
            if is_executable(&p) {
                return Some(p);
            }
        }
        if let Some(home) = dirs::home_dir() {
            for rel in HOME_RELATIVE {
                let p = home.join(rel);
                if is_executable(&p) {
                    return Some(p);
                }
            }
        }
        from_login_shell()
    }
}

#[cfg(windows)]
fn find_windows() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(v) = std::env::var_os(var) {
            roots.push(PathBuf::from(v));
        }
    }
    for root in roots {
        let p = root
            .join("Google")
            .join("Cloud SDK")
            .join("google-cloud-sdk")
            .join("bin")
            .join("gcloud.cmd");
        if p.is_file() {
            return Some(p);
        }
    }
    // PATH is inherited properly by scheduled tasks on Windows, so this is a
    // real fallback rather than a formality.
    from_path("gcloud.cmd")
}

/// Ask an interactive login shell, which is where `PATH` additions from
/// `.zshrc` and the SDK's own `path.bash.inc` actually live.
#[cfg(not(windows))]
fn from_login_shell() -> Option<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let out = std::process::Command::new(shell)
        .args(["-lc", "command -v gcloud"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let found = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let p = PathBuf::from(found);
    is_executable(&p).then_some(p)
}

#[cfg(windows)]
fn from_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn is_executable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The command a user should run to sign in again, as a displayable string.
pub fn login_command(gcloud: &Path) -> String {
    format!("{} auth login", quote_if_spaced(&gcloud.to_string_lossy()))
}

fn quote_if_spaced(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_with_spaces_are_quoted_for_display() {
        // The default Windows install lives under "Program Files (x86)", so
        // this is the normal case there, not an edge case.
        let p = Path::new(r"C:\Program Files (x86)\Google\Cloud SDK\bin\gcloud.cmd");
        assert!(login_command(p).starts_with('"'));
        assert!(login_command(Path::new("/usr/local/bin/gcloud")).starts_with('/'));
    }

    #[cfg(not(windows))]
    #[test]
    fn nothing_searched_needs_the_user_s_consent() {
        // Probing a consent gated directory raises a system prompt just by
        // asking whether a file is there. An app that claims to need no
        // permissions must not go looking in one.
        for candidate in HOME_RELATIVE {
            for gated in CONSENT_GATED {
                assert!(
                    !candidate.starts_with(gated),
                    "{candidate} sits inside {gated}, which macOS will prompt for"
                );
            }
        }
    }

    #[test]
    fn a_directory_is_not_an_executable() {
        assert!(!is_executable(Path::new("/")));
    }

    #[test]
    fn missing_paths_are_not_executable() {
        assert!(!is_executable(Path::new("/no/such/gcloud")));
    }
}
