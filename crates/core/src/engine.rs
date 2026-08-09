//! The state machine that ties probing, log scanning, and learning together.
//!
//! Deliberately free of threads, timers, and process spawning. The engine says
//! *what* should happen next and interprets what came back; the app crate does
//! the blocking work on a worker thread. That split is what lets the whole
//! sequence — including a session expiring while the machine was asleep — be
//! tested in microseconds.

use crate::config::ActiveConfig;
use crate::credentials::AdcFile;
use crate::estimate::{self, Estimate};
use crate::logs;
use crate::probe::ProbeOutcome;
use crate::state::State;
use crate::status::{AuthState, Status};
use chrono::{DateTime, Duration, Local};

/// Normal spacing between probes. Ten minutes bounds how stale "red" can be
/// while costing one cheap command per user per hour of idle time.
pub const PROBE_INTERVAL: Duration = Duration::seconds(600);

/// Used when the predicted expiry is close. A tighter interval near the
/// transition is what makes the *next* learned sample accurate, so the app gets
/// better at the one moment its measurement is worth taking.
pub const FAST_PROBE_INTERVAL: Duration = Duration::seconds(120);

/// How near the prediction counts as "close".
pub const FAST_PROBE_WINDOW: Duration = Duration::seconds(1800);

/// Spacing between rescans of gcloud's log directory.
pub const LOG_SCAN_INTERVAL: Duration = Duration::seconds(60);

/// After the user clicks "Sign in", probe hard so the icon turns green promptly
/// rather than up to ten minutes later.
pub const POST_LOGIN_INTERVAL: Duration = Duration::seconds(10);
pub const POST_LOGIN_DURATION: Duration = Duration::seconds(300);

/// What the engine wants done before the next tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    pub probe_user: bool,
    pub probe_adc: bool,
    pub rescan_logs: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Info,
    Warning,
}

/// Something the app should show the user.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Notify {
        title: String,
        body: String,
        urgency: Urgency,
    },
    /// State changed in a way worth persisting.
    Persist,
}

pub struct Engine {
    pub state: State,
    pub status: Status,
    last_probe_at: Option<DateTime<Local>>,
    last_scan_at: Option<DateTime<Local>>,
    /// Most recent moment the credential was known good. One half of the
    /// interval a session-length sample is measured across.
    last_valid_at: Option<DateTime<Local>>,
    fast_poll_until: Option<DateTime<Local>>,
}

impl Engine {
    pub fn new(state: State) -> Self {
        Self {
            state,
            status: Status::default(),
            last_probe_at: None,
            last_scan_at: None,
            last_valid_at: None,
            fast_poll_until: None,
        }
    }

    /// Recompute the estimate from whatever history exists.
    pub fn refresh_estimate(&mut self) {
        let gaps = estimate::gaps_between(&self.state.logins);
        self.status.estimate = estimate::estimate(&self.state.samples, &gaps);
        self.status.session_start = self.state.last_login();
    }

    pub fn set_environment(
        &mut self,
        gcloud_found: bool,
        config: Option<ActiveConfig>,
        adc_file: Option<AdcFile>,
    ) {
        self.status.gcloud_found = gcloud_found;
        self.status.config = config;
        self.status.adc_file = adc_file;
    }

    /// Decide what work is due.
    pub fn plan(&self, now: DateTime<Local>) -> Plan {
        if !self.status.gcloud_found {
            // Still rescan logs: gcloud may have been installed since startup,
            // and a login would be the first sign of it.
            return Plan {
                rescan_logs: self.scan_due(now),
                ..Default::default()
            };
        }
        let probe = self.probe_due(now);
        Plan {
            probe_user: probe,
            probe_adc: probe && self.state.settings.track_adc,
            rescan_logs: self.scan_due(now),
        }
    }

    fn scan_due(&self, now: DateTime<Local>) -> bool {
        match self.last_scan_at {
            None => true,
            Some(last) => now - last >= LOG_SCAN_INTERVAL,
        }
    }

    fn probe_due(&self, now: DateTime<Local>) -> bool {
        let Some(last) = self.last_probe_at else {
            return true;
        };
        now - last >= self.current_probe_interval(now)
    }

    /// The interval in force right now.
    pub fn current_probe_interval(&self, now: DateTime<Local>) -> Duration {
        if self.fast_poll_until.is_some_and(|until| now < until) {
            return POST_LOGIN_INTERVAL;
        }
        // Only tighten while the credential is believed good; once it is known
        // expired there is nothing left to measure.
        if self.status.auth == AuthState::Valid {
            if let Some(left) = self.status.remaining(now) {
                if left <= FAST_PROBE_WINDOW {
                    return FAST_PROBE_INTERVAL;
                }
            }
        }
        PROBE_INTERVAL
    }

    /// Called after the user asks to sign in.
    pub fn begin_fast_poll(&mut self, now: DateTime<Local>) {
        self.fast_poll_until = Some(now + POST_LOGIN_DURATION);
        self.last_probe_at = None;
    }

    /// Fold in the result of a user-credential probe.
    pub fn apply_user_probe(&mut self, outcome: ProbeOutcome, now: DateTime<Local>) -> Vec<Event> {
        self.last_probe_at = Some(now);
        self.status.checked_at = Some(now);
        let previous = self.status.auth.clone();
        let mut events = Vec::new();

        match outcome {
            ProbeOutcome::Valid => {
                self.status.auth = AuthState::Valid;
                self.status.probe_note = None;
                self.last_valid_at = Some(now);
                if previous != AuthState::Valid {
                    // Recovered, so stop hammering.
                    self.fast_poll_until = None;
                }
            }
            ProbeOutcome::Expired { detail } => {
                self.status.probe_note = None;
                if previous == AuthState::Valid {
                    if let Some(sample) = self.record_transition(now) {
                        events.push(Event::Persist);
                        let _ = sample;
                    }
                }
                self.status.auth = AuthState::Expired;
                events.extend(self.announce_expiry(now, &detail));
            }
            ProbeOutcome::Indeterminate { detail } => {
                // Hold the previous verdict. This is the rule that keeps the
                // icon worth believing on a flaky network.
                self.status.probe_note = Some(detail.clone());
                if matches!(previous, AuthState::Unknown(_)) {
                    self.status.auth = AuthState::Unknown(detail);
                }
            }
        }

        events.extend(self.check_thresholds(now));
        events
    }

    pub fn apply_adc_probe(&mut self, outcome: ProbeOutcome) {
        self.status.adc = Some(match outcome {
            ProbeOutcome::Valid => AuthState::Valid,
            ProbeOutcome::Expired { .. } => AuthState::Expired,
            ProbeOutcome::Indeterminate { detail } => AuthState::Unknown(detail),
        });
    }

    /// Fold in freshly scanned logins.
    pub fn apply_logins(&mut self, found: &[DateTime<Local>], now: DateTime<Local>) -> Vec<Event> {
        self.last_scan_at = Some(now);
        let previous_last = self.state.last_login();
        let changed = logs::merge(&mut self.state.logins, found);
        if !changed {
            return Vec::new();
        }
        self.state.trim();
        self.state.last_log_scan = Some(now);

        let new_last = self.state.last_login();
        let is_newer = match (previous_last, new_last) {
            (None, Some(_)) => true,
            (Some(a), Some(b)) => b > a,
            _ => false,
        };
        if is_newer {
            // A login happened. Whatever we announced about the old session no
            // longer applies, and the countdown restarts from here.
            self.state.begin_session();
            self.status.auth = AuthState::Valid;
            self.last_valid_at = new_last;
            self.last_probe_at = None; // confirm it for real, promptly
        }
        self.refresh_estimate();
        vec![Event::Persist]
    }

    /// Turn an observed valid → expired transition into a stored sample.
    fn record_transition(&mut self, now: DateTime<Local>) -> Option<f64> {
        let login = self.state.last_login()?;
        let last_valid = self.last_valid_at?;
        let sample = estimate::sample_from_transition(login, last_valid, now)?;
        self.state.record_sample(sample);
        self.refresh_estimate();
        Some(sample)
    }

    fn announce_expiry(&mut self, now: DateTime<Local>, detail: &str) -> Vec<Event> {
        if self.state.expiry_notified {
            return Vec::new();
        }
        if !self.state.settings.may_notify_at(now.time()) {
            // Suppressed, and deliberately *not* marked as announced, so the
            // first moment outside quiet hours still tells the user. The text
            // names no time, so it cannot go stale while it waits.
            return Vec::new();
        }
        self.state.expiry_notified = true;
        let who = self
            .status
            .config
            .as_ref()
            .and_then(|c| c.account.clone())
            .unwrap_or_else(|| "your account".into());
        let body = if detail.is_empty() {
            format!("{who} is signed out. Click to run gcloud auth login.")
        } else {
            format!("{who} is signed out. Click to run gcloud auth login.\n{detail}")
        };
        vec![
            Event::Notify {
                title: "gcloud session expired".into(),
                body,
                urgency: Urgency::Warning,
            },
            Event::Persist,
        ]
    }

    /// Emit an early warning if a threshold has just been crossed.
    fn check_thresholds(&mut self, now: DateTime<Local>) -> Vec<Event> {
        if self.status.auth != AuthState::Valid {
            return Vec::new();
        }
        let Some(left) = self.status.remaining(now) else {
            return Vec::new();
        };
        let minutes = left.num_minutes();
        let Some(threshold) = self
            .state
            .settings
            .threshold_crossed(minutes, &self.state.fired_thresholds)
        else {
            return Vec::new();
        };

        // Marked as fired whether or not it is shown. An early warning that
        // arrives hours late is worse than none: by then the number in it is
        // wrong, and the icon has been carrying the same information all along.
        self.state.fired_thresholds.push(threshold);
        if !self.state.settings.may_notify_at(now.time()) {
            return vec![Event::Persist];
        }
        vec![
            Event::Notify {
                title: "gcloud session expiring".into(),
                body: format!(
                    "About {} of gcloud auth left ({}).",
                    crate::status::long_duration(left),
                    self.status.estimate.source.label()
                ),
                urgency: Urgency::Info,
            },
            Event::Persist,
        ]
    }
}

/// Convenience for callers that just want the current estimate for a state.
pub fn estimate_for(state: &State) -> Estimate {
    estimate::estimate(&state.samples, &estimate::gaps_between(&state.logins))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{QuietHours, TimeOfDay};

    fn engine_with_session(hours_ago: f64) -> Engine {
        let mut state = State::default();
        state
            .logins
            .push(Local::now() - Duration::seconds((hours_ago * 3600.0) as i64));
        // Enough samples that the estimate is the measured 16h, not a fallback.
        for _ in 0..5 {
            state.record_sample(16.0);
        }
        let mut e = Engine::new(state);
        e.status.gcloud_found = true;
        e.refresh_estimate();
        e
    }

    #[test]
    fn first_tick_wants_everything() {
        let e = engine_with_session(1.0);
        let plan = e.plan(Local::now());
        assert!(plan.probe_user && plan.probe_adc && plan.rescan_logs);
    }

    #[test]
    fn probes_are_spaced_out() {
        let mut e = engine_with_session(1.0);
        let now = Local::now();
        e.apply_user_probe(ProbeOutcome::Valid, now);
        assert!(!e.plan(now + Duration::seconds(60)).probe_user);
        assert!(e.plan(now + PROBE_INTERVAL).probe_user);
    }

    #[test]
    fn probing_tightens_near_the_predicted_expiry() {
        let mut e = engine_with_session(15.9); // ~6 minutes left of 16h
        let now = Local::now();
        e.apply_user_probe(ProbeOutcome::Valid, now);
        assert_eq!(e.current_probe_interval(now), FAST_PROBE_INTERVAL);
        assert!(e.plan(now + FAST_PROBE_INTERVAL).probe_user);
    }

    #[test]
    fn probing_relaxes_once_the_session_is_known_gone() {
        let mut e = engine_with_session(15.9);
        let now = Local::now();
        e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: "invalid_grant".into(),
            },
            now,
        );
        // Nothing left to measure, so stop spending battery on it.
        assert_eq!(e.current_probe_interval(now), PROBE_INTERVAL);
    }

    #[test]
    fn adc_tracking_can_be_switched_off() {
        let mut e = engine_with_session(1.0);
        e.state.settings.track_adc = false;
        assert!(!e.plan(Local::now()).probe_adc);
    }

    #[test]
    fn a_transition_teaches_the_estimator() {
        let mut e = engine_with_session(16.0);
        let now = Local::now();
        e.apply_user_probe(ProbeOutcome::Valid, now - Duration::minutes(10));
        let before = e.state.samples.len();
        e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: "invalid_grant".into(),
            },
            now,
        );
        assert_eq!(e.state.samples.len(), before + 1);
        // Login was 16h ago and the last good probe 10 minutes ago, so the
        // midpoint lands 5 minutes back: 15h55m. Timestamping the failure
        // instead would have recorded the full 16h, overstating every future
        // session by the length of one probe interval.
        let recorded = *e.state.samples.last().unwrap();
        assert!((recorded - 15.92).abs() < 0.02, "got {recorded}");
    }

    #[test]
    fn an_indeterminate_probe_never_turns_the_dot_red() {
        let mut e = engine_with_session(1.0);
        let now = Local::now();
        e.apply_user_probe(ProbeOutcome::Valid, now);
        e.apply_user_probe(
            ProbeOutcome::Indeterminate {
                detail: "network unreachable".into(),
            },
            now + Duration::seconds(600),
        );
        assert_eq!(e.status.auth, AuthState::Valid);
        assert_eq!(e.status.probe_note.as_deref(), Some("network unreachable"));
        assert!(e.state.samples.iter().all(|s| (*s - 16.0).abs() < 1e-9));
    }

    #[test]
    fn expiry_notifies_once_per_session() {
        let mut e = engine_with_session(16.0);
        let now = Local::now();
        e.apply_user_probe(ProbeOutcome::Valid, now - Duration::minutes(10));
        let first = e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: String::new(),
            },
            now,
        );
        assert!(first.iter().any(|ev| matches!(ev, Event::Notify { .. })));

        let second = e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: String::new(),
            },
            now + Duration::seconds(600),
        );
        assert!(!second.iter().any(|ev| matches!(ev, Event::Notify { .. })));
    }

    #[test]
    fn a_new_login_resets_the_session() {
        let mut e = engine_with_session(16.0);
        let now = Local::now();
        e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: String::new(),
            },
            now,
        );
        assert!(e.state.expiry_notified);

        e.apply_logins(&[now], now);
        assert!(!e.state.expiry_notified, "a fresh login clears the flag");
        assert!(e.state.fired_thresholds.is_empty());
        assert_eq!(e.status.auth, AuthState::Valid);
        assert_eq!(e.status.session_start, Some(now));
    }

    #[test]
    fn rescanning_the_same_logins_changes_nothing() {
        let mut e = engine_with_session(2.0);
        let now = Local::now();
        let existing = e.state.logins.clone();
        assert!(e.apply_logins(&existing, now).is_empty());
    }

    #[test]
    fn thresholds_fire_as_the_session_runs_down() {
        let mut e = engine_with_session(14.5); // ~90 minutes left
        let now = Local::now();
        let events = e.apply_user_probe(ProbeOutcome::Valid, now);
        let notified: Vec<_> = events
            .iter()
            .filter_map(|ev| match ev {
                Event::Notify { body, .. } => Some(body.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(notified.len(), 1);
        assert!(notified[0].contains("1h"), "{}", notified[0]);
        assert!(notified[0].contains("measured"), "{}", notified[0]);
        assert_eq!(e.state.fired_thresholds, vec![120]);
    }

    #[test]
    fn quiet_hours_silence_warnings_but_still_consume_them() {
        let mut e = engine_with_session(14.5);
        // A window covering every possible clock time.
        e.state.settings.quiet_hours = Some(QuietHours {
            start: TimeOfDay::new(0, 0),
            end: TimeOfDay::new(23, 59),
        });
        let events = e.apply_user_probe(ProbeOutcome::Valid, Local::now());
        assert!(!events.iter().any(|ev| matches!(ev, Event::Notify { .. })));
        // Consumed, so it cannot resurface hours later quoting a stale number.
        assert_eq!(e.state.fired_thresholds, vec![120]);
    }

    #[test]
    fn quiet_hours_defer_the_expiry_notice_rather_than_dropping_it() {
        let mut e = engine_with_session(16.0);
        e.state.settings.quiet_hours = Some(QuietHours {
            start: TimeOfDay::new(0, 0),
            end: TimeOfDay::new(23, 59),
        });
        let now = Local::now();
        let events = e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: String::new(),
            },
            now,
        );
        assert!(!events.iter().any(|ev| matches!(ev, Event::Notify { .. })));
        // Not marked, because the user has still not been told.
        assert!(!e.state.expiry_notified);

        e.state.settings.quiet_hours = None;
        let later = e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: String::new(),
            },
            now,
        );
        assert!(later.iter().any(|ev| matches!(ev, Event::Notify { .. })));
    }

    #[test]
    fn notifications_can_be_disabled_entirely() {
        let mut e = engine_with_session(16.0);
        e.state.settings.notifications_enabled = false;
        let events = e.apply_user_probe(
            ProbeOutcome::Expired {
                detail: String::new(),
            },
            Local::now(),
        );
        assert!(!events.iter().any(|ev| matches!(ev, Event::Notify { .. })));
    }

    #[test]
    fn a_machine_asleep_through_both_marks_warns_once() {
        let mut e = engine_with_session(15.9); // ~6 minutes left, both passed
        let events = e.apply_user_probe(ProbeOutcome::Valid, Local::now());
        let count = events
            .iter()
            .filter(|ev| matches!(ev, Event::Notify { .. }))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn missing_gcloud_still_watches_for_a_login() {
        let mut e = engine_with_session(1.0);
        e.status.gcloud_found = false;
        let plan = e.plan(Local::now());
        assert!(!plan.probe_user);
        assert!(
            plan.rescan_logs,
            "installing gcloud mid-run must be noticed"
        );
    }

    #[test]
    fn signing_in_triggers_a_burst_of_probes() {
        let mut e = engine_with_session(1.0);
        let now = Local::now();
        e.apply_user_probe(ProbeOutcome::Valid, now);
        e.begin_fast_poll(now);
        assert_eq!(e.current_probe_interval(now), POST_LOGIN_INTERVAL);
        assert!(e.plan(now).probe_user);
        // And it stops on its own.
        assert_eq!(
            e.current_probe_interval(now + POST_LOGIN_DURATION + Duration::seconds(1)),
            PROBE_INTERVAL
        );
    }
}
