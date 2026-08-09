//! The model the UI draws, and the rules that turn it into a colour and a label.
//!
//! Kept free of any drawing so the same rules produce the macOS menu bar title,
//! the Windows tray bitmap, and the CLI's one-line output without three chances
//! to disagree with each other.

use crate::config::ActiveConfig;
use crate::credentials::AdcFile;
use crate::estimate::{Estimate, EstimateSource};
use chrono::{DateTime, Duration, Local};

/// What the probe established about a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    Valid,
    Expired,
    /// Never conflated with expiry. Carries why, for the tooltip.
    Unknown(String),
}

/// The five states the icon can be in, in order of severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warn,
    Soon,
    Expired,
    Unknown,
}

impl Level {
    /// sRGB, matching the Windows tray's palette so an existing user sees the
    /// same colours they are used to.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Level::Ok => (22, 163, 74),
            Level::Warn => (234, 179, 8),
            Level::Soon => (249, 115, 22),
            Level::Expired => (220, 38, 38),
            Level::Unknown => (107, 114, 128),
        }
    }

    /// Yellow and orange are far too light to carry white text.
    pub fn ink(self) -> (u8, u8, u8) {
        match self {
            Level::Warn | Level::Soon => (30, 30, 30),
            _ => (255, 255, 255),
        }
    }

    pub fn emoji(self) -> &'static str {
        match self {
            Level::Ok => "🟢",
            Level::Warn => "🟡",
            Level::Soon => "🟠",
            Level::Expired => "🔴",
            Level::Unknown => "⚪️",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub gcloud_found: bool,
    pub auth: AuthState,
    /// `None` when ADC tracking is off or no ADC file exists.
    pub adc: Option<AuthState>,
    pub adc_file: Option<AdcFile>,
    pub config: Option<ActiveConfig>,
    /// Start of the current session, from the login history.
    pub session_start: Option<DateTime<Local>>,
    pub estimate: Estimate,
    pub checked_at: Option<DateTime<Local>>,
    /// Why the last probe was inconclusive, if it was. Present alongside a
    /// still-valid `auth`, because an unreachable network does not revoke a
    /// credential — it only stops us confirming one.
    pub probe_note: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            gcloud_found: false,
            auth: AuthState::Unknown("starting up".into()),
            adc: None,
            adc_file: None,
            config: None,
            session_start: None,
            estimate: Estimate {
                hours: crate::estimate::FALLBACK_HOURS,
                source: EstimateSource::Default,
            },
            checked_at: None,
            probe_note: None,
        }
    }
}

impl Status {
    pub fn predicted_expiry(&self) -> Option<DateTime<Local>> {
        self.session_start
            .map(|s| crate::estimate::predicted_expiry(s, self.estimate.hours))
    }

    /// Time left before the *predicted* expiry. Negative once that moment has
    /// passed while the session is somehow still alive.
    pub fn remaining(&self, now: DateTime<Local>) -> Option<Duration> {
        self.predicted_expiry().map(|e| e - now)
    }

    pub fn level(&self, now: DateTime<Local>) -> Level {
        if !self.gcloud_found {
            return Level::Unknown;
        }
        match self.auth {
            AuthState::Expired => Level::Expired,
            AuthState::Unknown(_) => Level::Unknown,
            AuthState::Valid => match self.remaining(now) {
                // A valid session with no history is genuinely fine; there is
                // simply no countdown to show.
                None => Level::Ok,
                Some(left) => {
                    let mins = left.num_minutes();
                    if mins <= 30 {
                        Level::Soon
                    } else if mins <= 120 {
                        Level::Warn
                    } else {
                        Level::Ok
                    }
                }
            },
        }
    }

    /// The short string drawn on, or beside, the icon.
    pub fn icon_label(&self, now: DateTime<Local>) -> String {
        if !self.gcloud_found {
            return "?".into();
        }
        match self.auth {
            AuthState::Expired => "!".into(),
            AuthState::Unknown(_) => "?".into(),
            AuthState::Valid => match self.remaining(now) {
                None => "ok".into(),
                Some(left) => short_duration(left),
            },
        }
    }

    /// One line, for a tooltip or the CLI.
    pub fn summary(&self, now: DateTime<Local>) -> String {
        let base = self.summary_without_note(now);
        match &self.probe_note {
            Some(note) if self.auth == AuthState::Valid => {
                format!("{base} — last check inconclusive: {note}")
            }
            _ => base,
        }
    }

    fn summary_without_note(&self, now: DateTime<Local>) -> String {
        if !self.gcloud_found {
            return "gcloud is not installed on this machine".into();
        }
        match &self.auth {
            AuthState::Expired => "signed out — run gcloud auth login".into(),
            AuthState::Unknown(why) => format!("status unknown ({why})"),
            AuthState::Valid => match self.remaining(now) {
                None => "signed in (no login history yet)".into(),
                Some(left) if left.num_seconds() >= 0 => format!(
                    "signed in — about {} left ({})",
                    long_duration(left),
                    self.estimate.source.label()
                ),
                Some(left) => format!(
                    "signed in — {} past the estimate, still valid",
                    long_duration(-left)
                ),
            },
        }
    }
}

/// Compact form for a 32×32 icon: at most three characters.
///
/// Past the predicted expiry it reads "0m" rather than a negative number. The
/// session really is still valid, and the honest message is "any moment now",
/// which "0m" conveys in two characters.
pub fn short_duration(d: Duration) -> String {
    let mins = d.num_minutes();
    if mins <= 0 {
        return "0m".into();
    }
    if mins < 100 {
        return format!("{mins}m");
    }
    let hours = (d.num_seconds() as f64 / 3600.0).round() as i64;
    if hours >= 48 {
        return format!("{}d", d.num_days());
    }
    format!("{hours}h")
}

/// Readable form for menus and notifications.
pub fn long_duration(d: Duration) -> String {
    let total = d.num_minutes().max(0);
    let h = total / 60;
    let m = total % 60;
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// "3h ago", for timestamps in the panel.
pub fn ago(then: DateTime<Local>, now: DateTime<Local>) -> String {
    let d = now - then;
    if d.num_minutes() < 1 {
        return "just now".into();
    }
    if d.num_hours() < 1 {
        return format!("{}m ago", d.num_minutes());
    }
    if d.num_hours() < 48 {
        return format!("{}h ago", d.num_hours());
    }
    format!("{}d ago", d.num_days())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::estimate::Estimate;

    fn valid_status(hours_ago: f64, estimate_hours: f64) -> Status {
        Status {
            gcloud_found: true,
            auth: AuthState::Valid,
            session_start: Some(Local::now() - Duration::seconds((hours_ago * 3600.0) as i64)),
            estimate: Estimate {
                hours: estimate_hours,
                source: EstimateSource::Observed { count: 18 },
            },
            ..Default::default()
        }
    }

    #[test]
    fn levels_track_the_documented_bands() {
        let now = Local::now();
        assert_eq!(valid_status(1.0, 16.0).level(now), Level::Ok);
        assert_eq!(valid_status(14.5, 16.0).level(now), Level::Warn);
        assert_eq!(valid_status(15.8, 16.0).level(now), Level::Soon);
        assert_eq!(valid_status(20.0, 16.0).level(now), Level::Soon);
    }

    #[test]
    fn expiry_beats_any_countdown() {
        let mut s = valid_status(1.0, 16.0);
        s.auth = AuthState::Expired;
        assert_eq!(s.level(Local::now()), Level::Expired);
        assert_eq!(s.icon_label(Local::now()), "!");
    }

    #[test]
    fn a_network_failure_shows_unknown_not_expired() {
        let mut s = valid_status(1.0, 16.0);
        s.auth = AuthState::Unknown("network".into());
        assert_eq!(s.level(Local::now()), Level::Unknown);
        assert_eq!(s.icon_label(Local::now()), "?");
        assert!(s.summary(Local::now()).contains("network"));
    }

    #[test]
    fn missing_gcloud_is_unknown_whatever_else_is_true() {
        let mut s = valid_status(1.0, 16.0);
        s.gcloud_found = false;
        assert_eq!(s.level(Local::now()), Level::Unknown);
        assert!(s.summary(Local::now()).contains("not installed"));
    }

    #[test]
    fn valid_with_no_history_is_ok_and_says_so() {
        let mut s = valid_status(1.0, 16.0);
        s.session_start = None;
        assert_eq!(s.level(Local::now()), Level::Ok);
        assert_eq!(s.icon_label(Local::now()), "ok");
        assert!(s.summary(Local::now()).contains("no login history"));
    }

    #[test]
    fn icon_labels_fit_in_three_characters() {
        for (mins, want) in [
            (14 * 60, "14h"),
            (99, "99m"),
            (100, "2h"),
            (5, "5m"),
            (0, "0m"),
            (-30, "0m"),
            (72 * 60, "3d"),
        ] {
            let got = short_duration(Duration::minutes(mins));
            assert_eq!(got, want, "{mins} minutes");
            assert!(got.chars().count() <= 3, "{got} is too wide for the icon");
        }
    }

    #[test]
    fn long_duration_reads_naturally() {
        assert_eq!(long_duration(Duration::minutes(45)), "45m");
        assert_eq!(long_duration(Duration::minutes(120)), "2h");
        assert_eq!(long_duration(Duration::minutes(135)), "2h 15m");
    }

    #[test]
    fn past_the_estimate_stays_valid_and_says_so() {
        let s = valid_status(17.0, 16.0);
        let summary = s.summary(Local::now());
        assert!(summary.contains("past the estimate"), "{summary}");
        assert!(summary.contains("still valid"), "{summary}");
    }

    #[test]
    fn summary_names_the_evidence() {
        assert!(valid_status(1.0, 16.0)
            .summary(Local::now())
            .contains("n=18"));
    }

    #[test]
    fn warn_and_soon_use_dark_ink() {
        // White on yellow is unreadable at 16px; this is the guard for that.
        assert_eq!(Level::Warn.ink(), (30, 30, 30));
        assert_eq!(Level::Soon.ink(), (30, 30, 30));
        assert_eq!(Level::Ok.ink(), (255, 255, 255));
    }

    #[test]
    fn ago_reads_naturally() {
        let now = Local::now();
        assert_eq!(ago(now - Duration::seconds(10), now), "just now");
        assert_eq!(ago(now - Duration::minutes(30), now), "30m ago");
        assert_eq!(ago(now - Duration::hours(5), now), "5h ago");
        assert_eq!(ago(now - Duration::days(3), now), "3d ago");
    }
}
