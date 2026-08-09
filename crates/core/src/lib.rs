//! The engine behind GCloud Dot.
//!
//! Google does not tell a client when its reauth session ends, so this crate
//! does two separate jobs and never lets their results blur together:
//!
//! * [`probe`] establishes what is true right now by running a real command.
//! * [`estimate`] predicts when the session will end, from sessions already
//!   observed ending.
//!
//! Everything the user sees is labelled with which of those it came from. A red
//! icon is measured; a countdown is inferred, and says so.
//!
//! [`engine::Engine`] sequences the two without owning a thread, a timer, or a
//! process, which is what makes the whole behaviour testable.

pub mod config;
pub mod credentials;
pub mod engine;
pub mod estimate;
pub mod gcloud;
pub mod logs;
pub mod paths;
pub mod probe;
pub mod settings;
pub mod state;
pub mod status;

pub use engine::{Engine, Event, Plan, Urgency};
pub use estimate::{Estimate, EstimateSource};
pub use settings::Settings;
pub use state::State;
pub use status::{AuthState, Level, Status};

/// The version reported by `--version`, the panel, and the update check.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Load persisted state, adopting anything the previous apps left behind.
///
/// Returns the state and, when a migration happened, a line describing it so
/// the app can say so once rather than silently.
pub fn load_state() -> (State, Option<String>) {
    let path = paths::state_path();
    let mut state = State::load(&path);
    let migrated = state::migrate_legacy(&mut state);
    if migrated.is_some() {
        let _ = state.save(&path);
    }
    (state, migrated)
}

/// Read everything that can be learned from disk alone, with no subprocess.
///
/// Cheap enough to call on every menu open, which is what keeps the account and
/// project in the menu current without a spinner.
pub fn read_environment() -> (Option<config::ActiveConfig>, Option<credentials::AdcFile>) {
    let config = paths::gcloud_config_dir().and_then(|d| config::active(&d));
    let adc = paths::adc_path().and_then(|p| credentials::read_adc(&p));
    (config, adc)
}
