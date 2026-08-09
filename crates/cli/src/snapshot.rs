//! Composes the core pieces into one blocking read of the world.
//!
//! The tray keeps a long-lived [`gcloud_dot_core::Engine`] because it needs to
//! observe transitions over time. A CLI invocation has no history to keep, so
//! it does the same reads once and exits.

use chrono::Local;
use gcloud_dot_core::{
    engine, gcloud, logs, paths,
    probe::{self, Credential, ProbeOutcome},
    state::State,
    status::{AuthState, Status},
};
use std::time::Duration;

pub struct Snapshot {
    pub status: Status,
    pub state: State,
}

/// Read everything. `probe` decides whether to spend a subprocess and a network
/// round trip on ground truth, or report only what disk already knows.
pub fn take(run_probe: bool) -> Snapshot {
    let (mut state, _migrated) = gcloud_dot_core::load_state();
    let (config, adc_file) = gcloud_dot_core::read_environment();
    let gcloud_path = gcloud::find();

    // Pick up any logins since the last run so the countdown is current even if
    // the tray has never been started on this machine.
    if let Some(log_dir) = paths::gcloud_log_dir() {
        let found = logs::scan_logins(&log_dir, None);
        if logs::merge(&mut state.logins, &found) {
            state.trim();
            let _ = state.save(&paths::state_path());
        }
    }

    let mut status = Status {
        gcloud_found: gcloud_path.is_some(),
        config,
        adc_file,
        session_start: state.last_login(),
        estimate: engine::estimate_for(&state),
        ..Default::default()
    };

    if let (true, Some(path)) = (run_probe, gcloud_path.as_ref()) {
        let outcome = probe::run(path, Credential::User, Duration::from_secs(25));
        status.auth = match outcome {
            ProbeOutcome::Valid => AuthState::Valid,
            ProbeOutcome::Expired { .. } => AuthState::Expired,
            ProbeOutcome::Indeterminate { detail } => AuthState::Unknown(detail),
        };
        status.checked_at = Some(Local::now());

        if state.settings.track_adc && status.adc_file.is_some() {
            status.adc = Some(
                match probe::run(
                    path,
                    Credential::ApplicationDefault,
                    Duration::from_secs(25),
                ) {
                    ProbeOutcome::Valid => AuthState::Valid,
                    ProbeOutcome::Expired { .. } => AuthState::Expired,
                    ProbeOutcome::Indeterminate { detail } => AuthState::Unknown(detail),
                },
            );
        }
    } else if run_probe {
        status.auth = AuthState::Unknown("gcloud not found".into());
    } else {
        status.auth = AuthState::Unknown("not checked".into());
    }

    Snapshot { status, state }
}
