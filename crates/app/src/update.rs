//! Checking for and installing a new version, from the window.
//!
//! The decisions about what may be replaced and how live in
//! [`gcloud_dot_core::upgrade`], shared with the command line so the two cannot
//! disagree about whether Homebrew owns these files. What is here is only the
//! part that belongs to a window: run the work off the event loop, and report
//! each step back to it.

// Re-exported: the shape belongs to the window, but every caller reaches it
// through this module, which is where updates are otherwise dealt with.
pub use crate::panel::UpdateUi;
use gcloud_dot_core::upgrade::{self, Outcome};
use std::time::Duration;

pub use gcloud_dot_core::upgrade::RELEASES_PAGE;

/// Whether a newer release exists, without touching anything.
pub fn check() -> Option<String> {
    let release = upgrade::latest()?;
    upgrade::is_newer(&release.version, gcloud_dot_core::VERSION).then_some(release.version)
}

/// Spawn the check on a worker thread and hand the result back.
pub fn check_in_background<F: FnOnce(String) + Send + 'static>(delay: Duration, on_found: F) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if let Some(version) = check() {
            on_found(version);
        }
    });
}

/// Run the upgrade on a worker thread.
///
/// `progress` is called from that thread for each step and `done` once at the
/// end, so both have to reach the event loop rather than touch the window.
pub fn run_in_background<P, D>(progress: P, done: D)
where
    P: Fn(String) + Send + 'static,
    D: FnOnce(Result<Outcome, String>) + Send + 'static,
{
    std::thread::spawn(move || {
        let report = |line: &str| progress(line.to_string());
        done(upgrade::run(gcloud_dot_core::VERSION, &report));
    });
}

impl UpdateUi {
    /// Is an upgrade under way? While it is, the button must not start another.
    pub fn is_busy(&self) -> bool {
        matches!(self, UpdateUi::Working(_))
    }

    /// Turn a finished attempt into what the window should show.
    pub fn from_outcome(result: Result<Outcome, String>) -> Self {
        match result {
            Ok(Outcome::UpToDate(_)) => UpdateUi::Nothing,
            Ok(Outcome::Upgraded { to, .. }) => UpdateUi::Restarting(to),
            Ok(Outcome::Handed { to, .. }) => UpdateUi::Handed(to),
            Ok(Outcome::Manual { to, command, why }) => UpdateUi::Manual {
                version: to,
                command,
                why,
            },
            Err(e) => UpdateUi::Failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_replacement_promises_a_restart() {
        let ui = UpdateUi::from_outcome(Ok(Outcome::Upgraded {
            from: "1.0.5".into(),
            to: "1.0.6".into(),
        }));
        assert_eq!(ui, UpdateUi::Restarting("1.0.6".into()));
        assert!(!ui.is_busy());
    }

    #[test]
    fn a_package_managed_copy_is_told_the_command() {
        let ui = UpdateUi::from_outcome(Ok(Outcome::Manual {
            to: "1.0.6".into(),
            command: "sudo apt install ./gcloud-dot.deb".into(),
            why: "apt owns these files.".into(),
        }));
        match ui {
            UpdateUi::Manual { command, .. } => assert!(command.starts_with("sudo apt")),
            other => panic!("expected a command to run, got {other:?}"),
        }
    }

    #[test]
    fn finding_nothing_new_says_nothing() {
        // The window must not sprout a banner reading "up to date". The absence
        // of one already says that.
        let ui = UpdateUi::from_outcome(Ok(Outcome::UpToDate("1.0.5".into())));
        assert_eq!(ui, UpdateUi::Nothing);
    }

    #[test]
    fn a_failure_is_shown_rather_than_swallowed() {
        let ui = UpdateUi::from_outcome(Err("the download does not match its checksum".into()));
        assert!(matches!(ui, UpdateUi::Failed(_)));
        assert!(!ui.is_busy());
    }

    #[test]
    fn work_in_progress_blocks_a_second_attempt() {
        assert!(UpdateUi::Working("Downloading".into()).is_busy());
        assert!(!UpdateUi::Available("1.0.6".into()).is_busy());
    }
}
