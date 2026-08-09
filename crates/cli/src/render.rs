//! Output formatting.
//!
//! The JSON shape is an interface other people's prompts and CI steps will
//! depend on, so it is written out explicitly rather than derived from internal
//! types that are free to change.

use chrono::Local;
use gcloud_dot_core::{
    config::ActiveConfig,
    credentials::AdcKind,
    state::State,
    status::{ago, long_duration, AuthState, Level, Status},
};

fn level_name(l: Level) -> &'static str {
    match l {
        Level::Ok => "ok",
        Level::Warn => "warn",
        Level::Soon => "soon",
        Level::Expired => "expired",
        Level::Unknown => "unknown",
    }
}

fn auth_name(a: &AuthState) -> &'static str {
    match a {
        AuthState::Valid => "valid",
        AuthState::Expired => "expired",
        AuthState::Unknown(_) => "unknown",
    }
}

fn adc_kind_name(kind: &AdcKind) -> &'static str {
    match kind {
        AdcKind::UserCredentials => "user",
        AdcKind::ServiceAccount { .. } => "service_account",
        AdcKind::Other { .. } => "other",
    }
}

pub fn status_json(status: &Status, state: &State) -> String {
    let now = Local::now();
    let value = serde_json::json!({
        "version": gcloud_dot_core::VERSION,
        "gcloud_found": status.gcloud_found,
        "state": auth_name(&status.auth),
        "level": level_name(status.level(now)),
        "label": status.icon_label(now),
        "summary": status.summary(now),
        "account": status.config.as_ref().and_then(|c| c.account.clone()),
        "project": status.config.as_ref().and_then(|c| c.project.clone()),
        "configuration": status.config.as_ref().map(|c| c.name.clone()),
        "session_start": status.session_start,
        "predicted_expiry": status.predicted_expiry(),
        "seconds_remaining": status.remaining(now).map(|d| d.num_seconds()),
        "estimate_hours": status.estimate.hours,
        "estimate_source": status.estimate.source.label(),
        // The single most important field for anyone scripting against this:
        // whether the countdown is grounded in observation or is a guess.
        "estimate_is_measured": status.estimate.source.is_measured(),
        "samples": state.samples,
        "checked_at": status.checked_at,
        "probe_note": status.probe_note,
        "adc": status.adc_file.as_ref().map(|f| serde_json::json!({
            "kind": adc_kind_name(&f.kind),
            "state": status.adc.as_ref().map(auth_name),
            "file_modified": f.file_modified,
        })),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}

pub fn status_text(status: &Status, _state: &State) -> String {
    let now = Local::now();
    let level = status.level(now);
    let mut out = String::new();

    out.push_str(&format!("{} {}\n", level.emoji(), status.summary(now)));

    if let Some(cfg) = &status.config {
        if let Some(account) = &cfg.account {
            out.push_str(&format!("  account       {account}\n"));
        }
        if let Some(project) = &cfg.project {
            out.push_str(&format!("  project       {project}\n"));
        }
        out.push_str(&format!("  configuration {}\n", cfg.name));
    }

    if let Some(start) = status.session_start {
        out.push_str(&format!(
            "  last login    {} ({})\n",
            start.format("%a %b %-d, %H:%M"),
            ago(start, now)
        ));
    }
    // The predicted expiry only means something while the session is alive.
    // Once the probe has established it is gone, printing "still valid" beside
    // a red marker contradicts the line above it.
    if status.auth == AuthState::Valid {
        if let Some(expiry) = status.predicted_expiry() {
            let left = expiry - now;
            if left.num_seconds() >= 0 {
                out.push_str(&format!(
                    "  est. expiry   {} — {} left\n",
                    expiry.format("%a %H:%M"),
                    long_duration(left)
                ));
            } else {
                out.push_str(&format!(
                    "  est. expiry   {} — passed {}, still valid\n",
                    expiry.format("%a %H:%M"),
                    long_duration(-left)
                ));
            }
        }
    }
    if status.session_start.is_some() {
        out.push_str(&format!(
            "  session       {:.1}h ({})\n",
            status.estimate.hours,
            status.estimate.source.label()
        ));
    }

    if let Some(adc_file) = &status.adc_file {
        let state_str = match &status.adc {
            Some(a) => auth_name(a),
            None => "not checked",
        };
        let kind = match &adc_file.kind {
            AdcKind::UserCredentials => "user credentials".to_string(),
            AdcKind::ServiceAccount { client_email } => format!("service account {client_email}"),
            AdcKind::Other { kind } => kind.clone(),
        };
        out.push_str(&format!("  adc           {state_str} ({kind})\n"));
    }

    if let Some(checked) = status.checked_at {
        out.push_str(&format!("  last check    {}\n", checked.format("%H:%M:%S")));
    }
    out
}

pub fn history_text(state: &State) -> String {
    let now = Local::now();
    let mut out = String::new();

    if state.samples.is_empty() {
        out.push_str("No session lengths measured yet.\n");
        out.push_str("One is recorded each time a session is seen expiring.\n\n");
    } else {
        let e = gcloud_dot_core::engine::estimate_for(state);
        out.push_str(&format!(
            "Measured session length: {:.2}h ({})\n",
            e.hours,
            e.source.label()
        ));
        let mut sorted = state.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out.push_str(&format!(
            "  range {:.2}h – {:.2}h over {} samples\n\n",
            sorted.first().copied().unwrap_or_default(),
            sorted.last().copied().unwrap_or_default(),
            sorted.len()
        ));
    }

    if state.logins.is_empty() {
        out.push_str("No logins found in gcloud's logs.\n");
        return out;
    }

    out.push_str("Recent logins\n");
    let recent: Vec<_> = state.logins.iter().rev().take(12).collect();
    for (i, login) in recent.iter().enumerate() {
        // Gap to the previous login, which is what the fallback estimator uses
        // when no session has been observed ending.
        let gap = recent
            .get(i + 1)
            .map(|prev| format!("  +{:.1}h", (**login - **prev).num_minutes() as f64 / 60.0))
            .unwrap_or_default();
        out.push_str(&format!(
            "  {}{}  ({})\n",
            login.format("%Y-%m-%d %H:%M"),
            gap,
            ago(**login, now)
        ));
    }
    out
}

pub fn history_json(state: &State) -> String {
    let e = gcloud_dot_core::engine::estimate_for(state);
    let value = serde_json::json!({
        "estimate_hours": e.hours,
        "estimate_source": e.source.label(),
        "estimate_is_measured": e.source.is_measured(),
        "samples": state.samples,
        "logins": state.logins,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}

pub fn config_text(active: Option<&ActiveConfig>, all: &[String]) -> String {
    let mut out = String::new();
    match active {
        Some(a) => {
            out.push_str(&format!("active  {}\n", a.name));
            if let Some(acct) = &a.account {
                out.push_str(&format!("account {acct}\n"));
            }
            if let Some(p) = &a.project {
                out.push_str(&format!("project {p}\n"));
            }
        }
        None => out.push_str("no active configuration found\n"),
    }
    if !all.is_empty() {
        out.push_str("\nconfigurations\n");
        for name in all {
            let marker = if active.is_some_and(|a| &a.name == name) {
                "*"
            } else {
                " "
            };
            out.push_str(&format!("  {marker} {name}\n"));
        }
    }
    out
}

pub fn config_json(active: Option<&ActiveConfig>, all: &[String]) -> String {
    let value = serde_json::json!({
        "active": active.map(|a| serde_json::json!({
            "name": a.name,
            "account": a.account,
            "project": a.project,
        })),
        "configurations": all,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use gcloud_dot_core::estimate::{Estimate, EstimateSource};

    fn sample_status() -> Status {
        Status {
            gcloud_found: true,
            auth: AuthState::Valid,
            config: Some(ActiveConfig {
                name: "default".into(),
                account: Some("nic@glazkov.com".into()),
                project: Some("my-project".into()),
            }),
            session_start: Some(Local::now() - Duration::hours(2)),
            estimate: Estimate {
                hours: 16.06,
                source: EstimateSource::Observed { count: 18 },
            },
            checked_at: Some(Local::now()),
            ..Default::default()
        }
    }

    #[test]
    fn json_is_valid_and_carries_the_documented_keys() {
        let json = status_json(&sample_status(), &State::default());
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for key in [
            "version",
            "state",
            "level",
            "label",
            "summary",
            "account",
            "project",
            "seconds_remaining",
            "estimate_hours",
            "estimate_is_measured",
        ] {
            assert!(v.get(key).is_some(), "missing key {key}");
        }
        assert_eq!(v["state"], "valid");
        assert_eq!(v["level"], "ok");
        assert_eq!(v["estimate_is_measured"], true);
    }

    #[test]
    fn json_marks_an_unmeasured_estimate_as_such() {
        let mut s = sample_status();
        s.estimate = Estimate {
            hours: 20.0,
            source: EstimateSource::Default,
        };
        let v: serde_json::Value =
            serde_json::from_str(&status_json(&s, &State::default())).unwrap();
        assert_eq!(v["estimate_is_measured"], false);
    }

    #[test]
    fn text_output_names_the_account_and_the_evidence() {
        let text = status_text(&sample_status(), &State::default());
        assert!(text.contains("nic@glazkov.com"));
        assert!(text.contains("my-project"));
        assert!(text.contains("n=18"));
    }

    #[test]
    fn history_is_honest_when_nothing_has_been_measured() {
        let text = history_text(&State::default());
        assert!(text.contains("No session lengths measured yet"));
    }

    #[test]
    fn history_reports_the_measured_range() {
        let mut state = State::default();
        for s in [15.91, 16.07, 16.06, 16.08] {
            state.record_sample(s);
        }
        let text = history_text(&state);
        assert!(text.contains("15.91h – 16.08h"), "{text}");
    }

    #[test]
    fn config_text_marks_the_active_one() {
        let active = ActiveConfig {
            name: "work".into(),
            account: Some("a@b.com".into()),
            project: None,
        };
        let text = config_text(Some(&active), &["default".into(), "work".into()]);
        assert!(text.contains("  * work"), "{text}");
        assert!(text.contains("    default"), "{text}");
    }
}
