//! The predictive half of the app.
//!
//! Google publishes no session deadline to the client, so the only way to know
//! how long a session lasts is to watch one end. This module turns observations
//! into an estimate and, just as importantly, reports how good that estimate is
//! so the UI never presents a guess as a fact.

use chrono::{DateTime, Duration, Local};

/// Used before anything has been observed. Deliberately below Google's 16 hour
/// default: warning early is a minor annoyance, warning late is the failure
/// this app exists to prevent.
pub const FALLBACK_HOURS: f64 = 20.0;

/// A sample outside this range is not a session length. Below the floor it is
/// a double login; above the ceiling the machine was asleep across the real
/// expiry and the transition was observed days late.
const PLAUSIBLE_SAMPLE_HOURS: std::ops::Range<f64> = 1.0..168.0;

/// Gaps between logins only mean something inside the window where a forced
/// reauth is the likely cause. Shorter and it is someone switching accounts;
/// longer and they did not work that day.
const PLAUSIBLE_GAP_HOURS: std::ops::Range<f64> = 6.0..36.0;

/// Keeping more samples than this would let a policy change from six months ago
/// outvote what is true today.
pub const MAX_SAMPLES: usize = 20;

/// Where the current number came from. Shown verbatim in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstimateSource {
    /// Median of directly observed session lengths. The good case.
    Observed { count: usize },
    /// Mean of one or two observations. Better than nothing, not yet stable.
    FewObservations { count: usize },
    /// Inferred from how often you have had to log in before.
    LoginGaps { count: usize },
    /// Nothing to go on.
    Default,
}

impl EstimateSource {
    /// Short label for the menu and the panel.
    pub fn label(&self) -> String {
        match self {
            EstimateSource::Observed { count } => format!("measured, n={count}"),
            EstimateSource::FewObservations { count } => format!("measured, n={count}, settling"),
            EstimateSource::LoginGaps { count } => format!("inferred from {count} logins"),
            EstimateSource::Default => "no history yet".to_string(),
        }
    }

    /// Whether the estimate is trustworthy enough to show a precise countdown
    /// rather than a hedge.
    pub fn is_measured(&self) -> bool {
        matches!(self, EstimateSource::Observed { .. })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub hours: f64,
    pub source: EstimateSource,
}

/// Produce the current session-length estimate.
///
/// The ladder runs strongest evidence first: measured sessions, then a few
/// measured sessions, then the shape of your login history, then a constant.
pub fn estimate(samples: &[f64], login_gaps_hours: &[f64]) -> Estimate {
    let usable: Vec<f64> = samples
        .iter()
        .copied()
        .filter(|h| PLAUSIBLE_SAMPLE_HOURS.contains(h))
        .collect();

    if usable.len() >= 3 {
        return Estimate {
            hours: percentile(&usable, 0.5),
            source: EstimateSource::Observed {
                count: usable.len(),
            },
        };
    }
    if !usable.is_empty() {
        return Estimate {
            hours: usable.iter().sum::<f64>() / usable.len() as f64,
            source: EstimateSource::FewObservations {
                count: usable.len(),
            },
        };
    }

    let gaps: Vec<f64> = login_gaps_hours
        .iter()
        .copied()
        .filter(|h| PLAUSIBLE_GAP_HOURS.contains(h))
        .collect();
    if !gaps.is_empty() {
        return Estimate {
            // The 30th percentile rather than the median, because a gap is an
            // upper bound on the session: you log in some time *after* expiry,
            // never before it. Taking a low percentile removes most of that
            // bias without needing to model it.
            hours: percentile(&gaps, 0.3),
            source: EstimateSource::LoginGaps { count: gaps.len() },
        };
    }

    Estimate {
        hours: FALLBACK_HOURS,
        source: EstimateSource::Default,
    }
}

/// Nearest-rank percentile on a copy of the input.
///
/// For an even-length input the median is the lower of the two middle values,
/// not their mean. That is one sample's worth of conservatism in the direction
/// this app should always err.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    debug_assert!(!values.is_empty());
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (p * (sorted.len() - 1) as f64).floor() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Intervals between consecutive logins, in hours.
pub fn gaps_between(logins: &[DateTime<Local>]) -> Vec<f64> {
    logins
        .windows(2)
        .map(|w| (w[1] - w[0]).num_seconds() as f64 / 3600.0)
        .collect()
}

/// Turn an observed valid → expired transition into a session-length sample.
///
/// The true expiry lies somewhere between the last probe that succeeded and the
/// one that failed, and we have no way to narrow it further. Taking the
/// midpoint halves the worst-case error instead of biasing every sample late by
/// a full probe interval, which is what timestamping the failure would do.
///
/// Returns `None` when the arithmetic produces something that is not a
/// plausible session, so a laptop that slept through an expiry cannot teach the
/// estimator nonsense.
pub fn sample_from_transition(
    login: DateTime<Local>,
    last_valid: DateTime<Local>,
    first_invalid: DateTime<Local>,
) -> Option<f64> {
    if first_invalid < last_valid || last_valid < login {
        return None;
    }
    let midpoint = last_valid + (first_invalid - last_valid) / 2;
    let hours = (midpoint - login).num_seconds() as f64 / 3600.0;
    PLAUSIBLE_SAMPLE_HOURS
        .contains(&hours)
        .then(|| (hours * 100.0).round() / 100.0)
}

/// When the session is predicted to end.
pub fn predicted_expiry(login: DateTime<Local>, hours: f64) -> DateTime<Local> {
    login + Duration::seconds((hours * 3600.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 18 samples this machine's Windows tray had accumulated by August
    /// 2026. Every one lands within ten minutes of 16 hours, which is Google's
    /// documented default reauth policy, the estimator agreeing with the
    /// published number is the strongest evidence available that it works.
    const REAL_SAMPLES: [f64; 18] = [
        15.93, 16.07, 16.08, 15.91, 15.96, 15.91, 15.92, 16.07, 16.07, 15.93, 15.92, 16.06, 16.07,
        16.07, 16.07, 16.07, 16.06, 16.06,
    ];

    #[test]
    fn converges_on_the_real_world_data() {
        let e = estimate(&REAL_SAMPLES, &[]);
        assert_eq!(e.hours, 16.06);
        assert_eq!(e.source, EstimateSource::Observed { count: 18 });
        assert!((e.hours - 16.0).abs() < 0.1, "should agree with policy");
    }

    #[test]
    fn three_samples_switch_to_the_median() {
        let e = estimate(&[16.0, 16.1, 3.0], &[]);
        // The outlier is outvoted rather than averaged in.
        assert_eq!(e.hours, 16.0);
        assert!(e.source.is_measured());
    }

    #[test]
    fn one_or_two_samples_average_and_say_so() {
        let e = estimate(&[16.0, 18.0], &[]);
        assert_eq!(e.hours, 17.0);
        assert_eq!(e.source, EstimateSource::FewObservations { count: 2 });
        assert!(!e.source.is_measured());
    }

    #[test]
    fn implausible_samples_are_ignored_entirely() {
        // 0.2h is a double login; 400h is a machine that slept for weeks.
        let e = estimate(&[0.2, 400.0], &[]);
        assert_eq!(e.source, EstimateSource::Default);
        assert_eq!(e.hours, FALLBACK_HOURS);
    }

    #[test]
    fn falls_back_to_login_gaps() {
        let e = estimate(&[], &[16.5, 20.0, 24.0, 2.0, 100.0]);
        // 2.0 and 100.0 are filtered out; 30th percentile of the rest.
        assert_eq!(e.hours, 16.5);
        assert_eq!(e.source, EstimateSource::LoginGaps { count: 3 });
    }

    #[test]
    fn gap_estimate_is_conservative() {
        // A user who logs in every 16-24h should be warned nearer 16 than 24,
        // because the gap always overshoots the session it followed.
        let e = estimate(&[], &[16.0, 18.0, 20.0, 22.0, 24.0]);
        assert!(e.hours <= 18.0, "got {}", e.hours);
    }

    #[test]
    fn empty_everything_yields_the_default() {
        let e = estimate(&[], &[]);
        assert_eq!(e.hours, FALLBACK_HOURS);
        assert_eq!(e.source, EstimateSource::Default);
        assert_eq!(e.source.label(), "no history yet");
    }

    #[test]
    fn midpoint_halves_the_probe_interval_error() {
        let login = Local::now() - Duration::hours(17);
        let last_valid = login + Duration::hours(16);
        let first_invalid = last_valid + Duration::minutes(10);
        let s = sample_from_transition(login, last_valid, first_invalid).unwrap();
        // Timestamping the failure would have recorded 16.17h.
        assert_eq!(s, 16.08);
    }

    #[test]
    fn transition_rejects_impossible_orderings() {
        let now = Local::now();
        assert!(sample_from_transition(now, now + Duration::hours(1), now).is_none());
        assert!(sample_from_transition(now + Duration::hours(2), now, now).is_none());
    }

    #[test]
    fn transition_rejects_a_slept_through_expiry() {
        // Closed the laptop an hour after logging in, opened it three weeks
        // later. The midpoint lands at 250h, which is not a session length.
        let login = Local::now() - Duration::hours(500);
        let last_valid = login + Duration::hours(1);
        let first_invalid = login + Duration::hours(499);
        assert!(sample_from_transition(login, last_valid, first_invalid).is_none());
    }

    #[test]
    fn gaps_between_logins() {
        let base = Local::now();
        let logins = vec![base, base + Duration::hours(16), base + Duration::hours(40)];
        let g = gaps_between(&logins);
        assert_eq!(g.len(), 2);
        assert!((g[0] - 16.0).abs() < 1e-6);
        assert!((g[1] - 24.0).abs() < 1e-6);
    }

    #[test]
    fn percentile_endpoints() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 1.0), 4.0);
        assert_eq!(percentile(&v, 0.5), 2.0);
    }
}
