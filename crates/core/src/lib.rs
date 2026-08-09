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

/// Ask a running tray to exit.
///
/// Writes the request file the tray checks on each tick. Returns whether the
/// request could be written at all — not whether anything was listening.
pub fn request_quit() -> std::io::Result<()> {
    request_quit_at(&paths::quit_request_path())
}

fn request_quit_at(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, "quit")
}

/// Consume a pending quit request. True means the tray should exit now.
///
/// The file is removed before acting, so a tray that is killed between the two
/// does not find a stale request waiting at its next launch and exit again.
pub fn take_quit_request() -> bool {
    take_quit_request_at(&paths::quit_request_path())
}

fn take_quit_request_at(path: &std::path::Path) -> bool {
    if path.exists() {
        let _ = std::fs::remove_file(path);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_request_is_consumed_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("quit.request");

        // Nothing pending to begin with.
        assert!(!take_quit_request_at(&path));

        request_quit_at(&path).unwrap();
        assert!(take_quit_request_at(&path), "the request should be seen");
        // Consumed, so a tray restarting later does not exit immediately on a
        // request that was already answered.
        assert!(!take_quit_request_at(&path), "it must not fire twice");
    }
}
