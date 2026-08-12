//! Writes the details panel to an HTML file so it can be looked at.
//!
//! The panel is the one surface with real visual design in it, and the only way
//! to see it inside the app is to have a live gcloud session in a particular
//! state. This renders it from fixture data instead, which makes reviewing a
//! change to it a matter of opening a file.
//!
//! ```sh
//! cargo run -p gcloud-dot-app --example render_panel -- /tmp/panel.html
//! ```

/// Which update banner to draw, chosen by `GCLOUD_DOT_PANEL_UPDATE`.
///
/// The banner has six states and only one of them shows up in ordinary use, so
/// without a way to ask for the others they would be reviewed by reading the
/// template rather than by looking at them.
fn update_state() -> panel::UpdateUi {
    match std::env::var("GCLOUD_DOT_PANEL_UPDATE").unwrap_or_default().as_str() {
        "available" => panel::UpdateUi::Available("1.1.0".into()),
        "working" => panel::UpdateUi::Working("Downloading".into()),
        "restarting" => panel::UpdateUi::Restarting("1.1.0".into()),
        "handed" => panel::UpdateUi::Handed("1.1.0".into()),
        "manual" => panel::UpdateUi::Manual {
            version: "1.1.0".into(),
            command: "sudo apt install ./gcloud-dot_1.1.0_amd64.deb".into(),
            why: "This copy was installed by apt, which owns the files and needs root to replace them.".into(),
        },
        "failed" => panel::UpdateUi::Failed("The download does not match its published checksum.".into()),
        _ => panel::UpdateUi::Nothing,
    }
}

#[path = "../src/panel.rs"]
#[allow(dead_code)]
mod panel;

use chrono::{Duration, Local};
use gcloud_dot_core::{
    config::ActiveConfig,
    credentials::{AdcFile, AdcKind},
    estimate::{Estimate, EstimateSource},
    settings::Theme,
    status::{AuthState, Status},
    State,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "panel.html".to_string());

    let mut state = State::default();
    let base = Local::now() - Duration::hours(74);
    for i in 0..5 {
        state.logins.push(base + Duration::hours(i * 16 + i));
    }
    // The real distribution this app was built against: eighteen observations
    // clustered within ten minutes of sixteen hours.
    for s in [
        15.93, 16.07, 16.08, 15.91, 15.96, 15.91, 15.92, 16.07, 16.07, 15.93, 15.92, 16.06, 16.07,
        16.07, 16.07, 16.07, 16.06, 16.06,
    ] {
        state.record_sample(s);
    }

    let status = Status {
        gcloud_found: true,
        auth: AuthState::Valid,
        adc: Some(AuthState::Valid),
        adc_file: Some(AdcFile {
            kind: AdcKind::UserCredentials,
            file_modified: Some(Local::now() - Duration::days(3)),
        }),
        config: Some(ActiveConfig {
            name: "default".into(),
            account: Some("nic@glazkov.com".into()),
            project: Some("chalkboard-493217".into()),
        }),
        session_start: state.last_login(),
        estimate: Estimate {
            hours: 16.06,
            source: EstimateSource::Observed { count: 18 },
        },
        checked_at: Some(Local::now()),
        probe_note: None,
    };

    // Rendered in each theme so a change can be reviewed in both without
    // toggling the whole machine.
    for (theme, suffix) in [
        (Theme::System, ""),
        (Theme::Light, "-light"),
        (Theme::Dark, "-dark"),
    ] {
        let html = panel::document(&panel::view(&status, &state, &update_state()), theme);
        let path = out.replace(".html", &format!("{suffix}.html"));
        std::fs::write(&path, html).expect("could not write the file");
        println!("wrote {path}");
    }
}
