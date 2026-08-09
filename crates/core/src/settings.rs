//! User-facing preferences.
//!
//! Every default here is chosen so that a user who never opens settings gets
//! the behaviour the app is arguing for: warned early, never interrupted for a
//! network blip, and never acted upon without asking.

use chrono::{NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

/// Appearance of the details window.
///
/// `System` is the default and the right answer for almost everyone; the two
/// overrides exist because a menu bar app is often the one window left open on
/// a machine whose global theme is set for something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    /// The value for the document's `data-theme` attribute. Empty means "let
    /// `prefers-color-scheme` decide", which is what the stylesheet falls back
    /// to when the attribute is absent.
    pub fn attr(self) -> &'static str {
        match self {
            Theme::System => "",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "Match system",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }

    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Minutes-remaining marks at which to notify, largest first. Each fires at
    /// most once per session.
    pub warn_at_minutes: Vec<i64>,
    pub notifications_enabled: bool,
    /// Suppress notifications between these times. Stored as "HH:MM" strings so
    /// the file stays legible to a human editing it by hand.
    pub quiet_hours: Option<QuietHours>,
    /// macOS only: draw the countdown as text beside the menu bar dot. On
    /// Windows and Linux the countdown is drawn into the icon itself, because
    /// neither platform has a text slot in the tray.
    pub show_countdown_text: bool,
    /// Track application-default credentials alongside the user login.
    pub track_adc: bool,
    /// Warn when a service account key file is older than this. Zero disables.
    pub sa_key_warn_days: i64,
    pub launch_at_login: bool,
    pub check_for_updates: bool,
    /// Appearance of the details window, on every platform.
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Two hours is enough time to finish what you are doing; thirty
            // minutes is enough to stop and re-auth before it bites. These are
            // the same bands the macOS dot used for yellow and orange.
            warn_at_minutes: vec![120, 30],
            notifications_enabled: true,
            quiet_hours: None,
            show_countdown_text: true,
            track_adc: true,
            sa_key_warn_days: 90,
            launch_at_login: true,
            check_for_updates: true,
            theme: Theme::System,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    /// "HH:MM", inclusive.
    pub start: TimeOfDay,
    /// "HH:MM", exclusive.
    pub end: TimeOfDay,
}

impl QuietHours {
    /// Whether a given local time falls inside the quiet window.
    ///
    /// Windows that wrap past midnight are the normal case, 22:00 to 07:00,
    /// so the wrapping branch is the one to get right.
    pub fn contains(&self, now: NaiveTime) -> bool {
        let now = now.num_seconds_from_midnight();
        let start = self.start.seconds();
        let end = self.end.seconds();
        if start == end {
            return false; // A zero-width window silences nothing.
        }
        if start < end {
            now >= start && now < end
        } else {
            now >= start || now < end
        }
    }
}

/// Minutes since midnight, serialised as "HH:MM".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u32,
    pub minute: u32,
}

impl TimeOfDay {
    pub fn new(hour: u32, minute: u32) -> Self {
        Self {
            hour: hour.min(23),
            minute: minute.min(59),
        }
    }
    fn seconds(&self) -> u32 {
        self.hour * 3600 + self.minute * 60
    }
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl Serialize for TimeOfDay {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        let (h, m) = raw
            .split_once(':')
            .ok_or_else(|| serde::de::Error::custom("expected HH:MM"))?;
        Ok(TimeOfDay::new(
            h.trim().parse().map_err(serde::de::Error::custom)?,
            m.trim().parse().map_err(serde::de::Error::custom)?,
        ))
    }
}

impl Settings {
    /// Whether a notification may be shown right now.
    ///
    /// Quiet hours suppress the *notification*, never the state itself: the
    /// icon still turns red at 3am, it does not make noise about it.
    pub fn may_notify_at(&self, now: NaiveTime) -> bool {
        if !self.notifications_enabled {
            return false;
        }
        match &self.quiet_hours {
            Some(q) => !q.contains(now),
            None => true,
        }
    }

    /// The largest threshold that has just been crossed, given how many minutes
    /// remain and what has already been announced this session.
    pub fn threshold_crossed(&self, minutes_left: i64, already_fired: &[i64]) -> Option<i64> {
        let mut candidates: Vec<i64> = self
            .warn_at_minutes
            .iter()
            .copied()
            .filter(|t| minutes_left <= *t && !already_fired.contains(t))
            .collect();
        // Firing the *largest* unfired threshold means a machine that was
        // asleep through both marks emits one notification, not two.
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn defaults_are_the_documented_bands() {
        let s = Settings::default();
        assert_eq!(s.warn_at_minutes, vec![120, 30]);
        assert!(s.notifications_enabled);
        assert!(s.quiet_hours.is_none());
    }

    #[test]
    fn quiet_hours_across_midnight() {
        let q = QuietHours {
            start: TimeOfDay::new(22, 0),
            end: TimeOfDay::new(7, 0),
        };
        assert!(q.contains(t(23, 30)));
        assert!(q.contains(t(3, 0)));
        assert!(q.contains(t(22, 0)), "start is inclusive");
        assert!(!q.contains(t(7, 0)), "end is exclusive");
        assert!(!q.contains(t(12, 0)));
    }

    #[test]
    fn quiet_hours_within_one_day() {
        let q = QuietHours {
            start: TimeOfDay::new(9, 0),
            end: TimeOfDay::new(17, 30),
        };
        assert!(q.contains(t(12, 0)));
        assert!(!q.contains(t(8, 59)));
        assert!(!q.contains(t(17, 30)));
    }

    #[test]
    fn zero_width_quiet_hours_silence_nothing() {
        let q = QuietHours {
            start: TimeOfDay::new(9, 0),
            end: TimeOfDay::new(9, 0),
        };
        assert!(!q.contains(t(9, 0)));
    }

    #[test]
    fn notifications_respect_quiet_hours_and_the_master_switch() {
        let mut s = Settings {
            quiet_hours: Some(QuietHours {
                start: TimeOfDay::new(22, 0),
                end: TimeOfDay::new(7, 0),
            }),
            ..Default::default()
        };
        assert!(!s.may_notify_at(t(2, 0)));
        assert!(s.may_notify_at(t(10, 0)));

        s.notifications_enabled = false;
        assert!(!s.may_notify_at(t(10, 0)));
    }

    #[test]
    fn thresholds_fire_once_each() {
        let s = Settings::default();
        assert_eq!(s.threshold_crossed(119, &[]), Some(120));
        assert_eq!(s.threshold_crossed(100, &[120]), None);
        assert_eq!(s.threshold_crossed(29, &[120]), Some(30));
        assert_eq!(s.threshold_crossed(5, &[120, 30]), None);
    }

    #[test]
    fn a_sleeping_machine_emits_one_notification_not_two() {
        // Laptop closed at 3h remaining, opened at 10m remaining. Both marks
        // were passed, but the user should hear about it once.
        let s = Settings::default();
        assert_eq!(s.threshold_crossed(10, &[]), Some(120));
    }

    #[test]
    fn time_of_day_round_trips_through_json() {
        let q = QuietHours {
            start: TimeOfDay::new(22, 5),
            end: TimeOfDay::new(7, 0),
        };
        let json = serde_json::to_string(&q).unwrap();
        assert!(json.contains("22:05"), "{json}");
        assert_eq!(serde_json::from_str::<QuietHours>(&json).unwrap(), q);
    }

    #[test]
    fn settings_tolerate_missing_fields() {
        // Forward compatibility: an older file must not wipe newer defaults.
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
    }
}
