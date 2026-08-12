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
pub mod proc;
pub mod settings;
pub mod state;
pub mod status;
pub mod upgrade;

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
/// request could be written at all, not whether anything was listening.
pub fn request_quit() -> std::io::Result<()> {
    request_quit_at(&paths::quit_request_path())
}

fn request_quit_at(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, "quit")
}

/// How long a quit request stays meaningful.
///
/// A quit request is a message to a tray that is running *now*. If nothing has
/// consumed it within this window then nothing was listening, and acting on it
/// later means a tray launched tomorrow exiting on an instruction meant for the
/// one running yesterday.
///
/// This is not hypothetical. `gcloud-dot quit` deletes the file itself when no
/// tray answers, but only if it lives long enough to do so; kill that command
/// while it is waiting, or lose the machine to a sleep or a reboot, and the
/// file outlives everything. Every launch afterwards then exits within one
/// tick, and the app looks broken in a way that leaves no trace of why.
const QUIT_REQUEST_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Consume a pending quit request. True means the tray should exit now.
///
/// The file is removed before acting, so a tray killed between the two does not
/// find the same request waiting at its next launch.
pub fn take_quit_request() -> bool {
    take_quit_request_at(&paths::quit_request_path(), std::time::SystemTime::now())
}

fn take_quit_request_at(path: &std::path::Path, now: std::time::SystemTime) -> bool {
    let Ok(written) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    // Removed either way. A request too old to act on is exactly the one that
    // must not be left lying there for the next launch to find.
    let _ = std::fs::remove_file(path);
    // A clock that has moved backwards leaves the age unknowable. Acting is the
    // better guess: the request was written by someone who wanted this to stop,
    // and the file is gone now regardless, so at worst it costs one restart.
    now.duration_since(written)
        .map(|age| age <= QUIT_REQUEST_TTL)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, SystemTime};

    #[test]
    fn a_quit_request_is_consumed_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("quit.request");
        let now = SystemTime::now();

        // Nothing pending to begin with.
        assert!(!take_quit_request_at(&path, now));

        request_quit_at(&path).unwrap();
        assert!(
            take_quit_request_at(&path, now),
            "the request should be seen"
        );
        // Consumed, so a tray restarting later does not exit immediately on a
        // request that was already answered.
        assert!(!take_quit_request_at(&path, now), "it must not fire twice");
    }

    #[test]
    fn a_stale_quit_request_is_ignored_and_cleared() {
        // Reproduces a real failure: `gcloud-dot quit` was killed before it
        // could clean up after itself, and the file it left behind stopped
        // every tray started afterwards, within a tick of launching.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quit.request");
        request_quit_at(&path).unwrap();

        let much_later = SystemTime::now() + Duration::from_secs(3600);
        assert!(
            !take_quit_request_at(&path, much_later),
            "an hour old request was meant for a tray that is long gone"
        );
        assert!(
            !path.exists(),
            "and it must be cleared, or it stops the next launch too"
        );
    }

    #[test]
    fn a_request_still_within_the_window_is_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quit.request");
        request_quit_at(&path).unwrap();
        let soon = SystemTime::now() + Duration::from_secs(5);
        assert!(take_quit_request_at(&path, soon));
    }
}
