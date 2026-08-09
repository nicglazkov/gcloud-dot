//! Everything the app remembers between runs.
//!
//! One JSON file, written atomically. It is meant to be readable and editable
//! by hand: someone who wants to know why the app thinks their session is 16
//! hours long should be able to open this and see the eighteen numbers it is
//! averaging.

use crate::estimate::MAX_SAMPLES;
use crate::settings::Settings;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Enough history to compute gaps without letting a year-old habit outvote
/// this week's.
pub const MAX_LOGINS: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Bumped only for changes the loader cannot infer. Present from v1 so a
    /// future migration has something to branch on.
    pub version: u32,
    /// Completed `gcloud auth login` times, oldest first.
    pub logins: Vec<DateTime<Local>>,
    /// Observed session lengths in hours.
    pub samples: Vec<f64>,
    /// Warning thresholds already announced for the current session.
    pub fired_thresholds: Vec<i64>,
    /// Whether expiry has been announced for the current session.
    pub expiry_notified: bool,
    pub last_log_scan: Option<DateTime<Local>>,
    pub settings: Settings,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            logins: Vec::new(),
            samples: Vec::new(),
            fired_thresholds: Vec::new(),
            expiry_notified: false,
            last_log_scan: None,
            settings: Settings::default(),
        }
    }
}

impl State {
    pub fn last_login(&self) -> Option<DateTime<Local>> {
        self.logins.last().copied()
    }

    /// Called when a new login is detected. Everything announced about the old
    /// session becomes irrelevant.
    pub fn begin_session(&mut self) {
        self.fired_thresholds.clear();
        self.expiry_notified = false;
    }

    pub fn record_sample(&mut self, hours: f64) {
        self.samples.push(hours);
        if self.samples.len() > MAX_SAMPLES {
            let excess = self.samples.len() - MAX_SAMPLES;
            self.samples.drain(0..excess);
        }
    }

    pub fn trim(&mut self) {
        if self.logins.len() > MAX_LOGINS {
            let excess = self.logins.len() - MAX_LOGINS;
            self.logins.drain(0..excess);
        }
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(strip_bom(&raw)).ok())
            .unwrap_or_default()
    }

    /// Write via a temporary file in the same directory, then rename.
    ///
    /// A tray app is killed at logout mid-write often enough that a torn state
    /// file is a real failure mode, and it would silently cost the user every
    /// sample they had accumulated.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)
    }
}

/// Remove a UTF-8 byte order mark, which is not valid JSON.
///
/// Windows PowerShell 5.1 writes one for `Set-Content -Encoding UTF8`, so every
/// state file the PowerShell tray ever wrote begins with U+FEFF. `read_to_string`
/// accepts those bytes happily — they are well-formed UTF-8 — and then
/// `serde_json` rejects the document at byte zero. The failure is silent,
/// because a legacy file that cannot be parsed is indistinguishable from one
/// that is not there.
///
/// Notepad does the same thing, so this protects hand-edited files of our own.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// The shape written by the PowerShell tray this app replaces.
#[derive(Debug, Deserialize)]
struct LegacyWindowsState {
    #[serde(default)]
    logins: Vec<DateTime<Local>>,
    #[serde(default)]
    samples: Vec<f64>,
}

/// Adopt whatever the previous apps had learned.
///
/// Session samples cost real wall-clock time to gather — a user with eighteen
/// of them has been running the old tray for three weeks. Throwing that away on
/// upgrade would make the new app worse than the one it replaces on day one.
pub fn migrate_legacy(state: &mut State) -> Option<String> {
    if !state.logins.is_empty() || !state.samples.is_empty() {
        return None; // Already have our own history; never overwrite it.
    }

    if let Some(path) = crate::paths::legacy_windows_state_path() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(legacy) = serde_json::from_str::<LegacyWindowsState>(strip_bom(&raw)) {
                if !legacy.logins.is_empty() || !legacy.samples.is_empty() {
                    state.logins = legacy.logins;
                    state.samples = legacy.samples;
                    state.trim();
                    if state.samples.len() > MAX_SAMPLES {
                        let excess = state.samples.len() - MAX_SAMPLES;
                        state.samples.drain(0..excess);
                    }
                    return Some(format!(
                        "imported {} logins and {} session samples from GcloudAuthTray",
                        state.logins.len(),
                        state.samples.len()
                    ));
                }
            }
        }
    }

    if let Some(hours) = legacy_macos_observed_hours() {
        state.record_sample(hours);
        return Some(format!(
            "imported one measured session length ({hours:.1}h) from GCloud Dot's preferences"
        ));
    }

    None
}

/// The Swift app stored its one measurement in `UserDefaults`, which on disk is
/// a binary plist. Asking `defaults` to read it is cheaper and far less
/// fragile than parsing that format for a single number.
#[cfg(target_os = "macos")]
fn legacy_macos_observed_hours() -> Option<f64> {
    let out = std::process::Command::new("defaults")
        .args(["read", "com.nic.gclouddot", "ObservedSessionHours"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let hours: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    (hours > 1.0 && hours < 168.0).then_some(hours)
}

#[cfg(not(target_os = "macos"))]
fn legacy_macos_observed_hours() -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = State::default();
        s.logins.push(Local::now());
        s.record_sample(16.06);
        s.save(&path).unwrap();
        assert_eq!(State::load(&path), s);
    }

    #[test]
    fn save_creates_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("state.json");
        State::default().save(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        State::default().save(&path).unwrap();
        let stray: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(stray.is_empty(), "temp file was not renamed away");
    }

    #[test]
    fn a_corrupt_file_yields_defaults_rather_than_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert_eq!(State::load(&path), State::default());
    }

    #[test]
    fn a_missing_file_yields_defaults() {
        assert_eq!(
            State::load(Path::new("/no/such/state.json")),
            State::default()
        );
    }

    #[test]
    fn samples_are_capped_keeping_the_newest() {
        let mut s = State::default();
        for i in 0..30 {
            s.record_sample(i as f64 + 1.0);
        }
        assert_eq!(s.samples.len(), MAX_SAMPLES);
        assert_eq!(*s.samples.last().unwrap(), 30.0);
        assert_eq!(s.samples[0], 11.0);
    }

    #[test]
    fn logins_are_capped_keeping_the_newest() {
        let mut s = State::default();
        let base = Local::now();
        for i in 0..250 {
            s.logins.push(base + Duration::hours(i));
        }
        s.trim();
        assert_eq!(s.logins.len(), MAX_LOGINS);
        assert_eq!(s.logins.last().unwrap(), &(base + Duration::hours(249)));
    }

    #[test]
    fn a_new_session_clears_what_was_announced() {
        let mut s = State {
            fired_thresholds: vec![120, 30],
            expiry_notified: true,
            ..Default::default()
        };
        s.begin_session();
        assert!(s.fired_thresholds.is_empty());
        assert!(!s.expiry_notified);
    }

    #[test]
    fn parses_the_real_legacy_windows_file() {
        // Verbatim shape of %LOCALAPPDATA%\GcloudAuthTray\state.json, including
        // the offset timestamps PowerShell's ConvertTo-Json emits.
        let raw = r#"{
            "logins": ["2026-08-06T18:21:00.2059236-07:00", "2026-08-07T12:26:53.8758608-07:00"],
            "samples": [15.93, 16.07, 16.08],
            "warned": true,
            "alerted": true,
            "lastLogScan": "2026-08-08T04:35:19.2872523-07:00"
        }"#;
        let legacy: LegacyWindowsState = serde_json::from_str(raw).unwrap();
        assert_eq!(legacy.logins.len(), 2);
        assert_eq!(legacy.samples, vec![15.93, 16.07, 16.08]);
    }

    #[test]
    fn parses_a_legacy_file_with_a_byte_order_mark() {
        // Windows PowerShell 5.1 writes a UTF-8 BOM for `Set-Content -Encoding
        // UTF8`, so every file the previous tray produced starts with one.
        // Without stripping it, serde_json fails at byte zero and the whole
        // migration is skipped in silence — which is exactly what happened on
        // a real machine holding eighteen hard-won samples.
        let raw = "\u{feff}{\"logins\":[],\"samples\":[16.06,16.07,15.91]}";
        let legacy: LegacyWindowsState = serde_json::from_str(strip_bom(raw)).unwrap();
        assert_eq!(legacy.samples.len(), 3);
    }

    #[test]
    fn our_own_state_survives_a_byte_order_mark() {
        // Someone opening state.json in Notepad and saving it adds one too.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = State::default();
        s.record_sample(16.06);
        let json = serde_json::to_string(&s).unwrap();
        std::fs::write(&path, format!("\u{feff}{json}")).unwrap();
        assert_eq!(State::load(&path).samples, vec![16.06]);
    }

    #[test]
    fn strip_bom_leaves_ordinary_text_alone() {
        assert_eq!(strip_bom("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_bom("\u{feff}{}"), "{}");
    }

    #[test]
    fn migration_never_overwrites_existing_history() {
        let mut s = State::default();
        s.record_sample(16.0);
        let before = s.clone();
        assert!(migrate_legacy(&mut s).is_none());
        assert_eq!(s, before);
    }
}
