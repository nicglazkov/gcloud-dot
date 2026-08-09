//! Where things live on each platform.
//!
//! Two independent trees matter here: gcloud's own configuration directory,
//! which we only ever read, and our state directory, which we own.

use std::path::PathBuf;

/// gcloud's configuration directory.
///
/// `CLOUDSDK_CONFIG` overrides the default everywhere and is how people run
/// several isolated SDK installs side by side, so it is checked first. Missing
/// that, gcloud uses `%APPDATA%\gcloud` on Windows and `~/.config/gcloud`
/// elsewhere, note that it is `.config` even on macOS, where an application
/// would normally use Application Support.
pub fn gcloud_config_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CLOUDSDK_CONFIG") {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("gcloud"))
    } else {
        dirs::home_dir().map(|h| h.join(".config").join("gcloud"))
    }
}

/// The directory gcloud writes its per-invocation logs into.
pub fn gcloud_log_dir() -> Option<PathBuf> {
    gcloud_config_dir().map(|c| c.join("logs"))
}

/// Path of the application-default credentials file, which is a different
/// credential with a different lifetime from the user login.
pub fn adc_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS") {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    gcloud_config_dir().map(|c| c.join("application_default_credentials.json"))
}

/// Our own state directory, following each platform's convention.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("GCloudDot")
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("GCloudDot")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gcloud-dot")
    }
}

pub fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

/// A file the CLI drops to ask a running tray to exit.
///
/// A file rather than a signal or a socket because it is the only mechanism
/// that behaves the same on all three platforms with no dependency, and because
/// the tray is already waking on a timer, it costs one `exists` call per tick.
///
/// This matters more than it sounds: if the menu bar is full, macOS stops
/// drawing the icon, and the menu holding "Quit" becomes unreachable. Without
/// this the only way out would be Activity Monitor.
pub fn quit_request_path() -> PathBuf {
    data_dir().join("quit.request")
}

/// State written by the PowerShell tray this app replaces. Read once, on first
/// run, so an existing user keeps the session samples they already paid for in
/// wall-clock time.
pub fn legacy_windows_state_path() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    dirs::data_local_dir().map(|d| d.join("GcloudAuthTray").join("state.json"))
}

/// Preferences written by the Swift menu bar app this app replaces.
pub fn legacy_macos_prefs_path() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    dirs::home_dir().map(|h| h.join("Library/Preferences/com.nic.gclouddot.plist"))
}
