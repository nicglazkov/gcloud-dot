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

/// How often to look, after the first look.
///
/// This app is meant to sit in the menu bar for weeks at a time. Checking only
/// at launch means a release that lands on a Tuesday is not mentioned until
/// something happens to restart the tray, which for a well behaved background
/// app might be never.
const RECHECK: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the worker wakes to see whether it should be doing anything.
///
/// Short, so that switching the setting back on produces an answer while the
/// menu the user just clicked is still in mind, rather than up to a day later.
const POLL: Duration = Duration::from_secs(60);

/// Keep checking on a worker thread, reporting whatever it finds.
///
/// One thread for the life of the process, whatever the setting does. The
/// obvious alternative, starting a checker when the setting goes on, gets both
/// halves wrong: switching it off leaves the running thread checking and still
/// reporting, and switching it on and off repeatedly leaves one behind each
/// time.
///
/// `on_found` fires on every check that finds a newer release, including
/// repeats of one already reported. Deciding what is worth saying out loud
/// belongs to the caller, which is the only side that knows what the user has
/// already been told.
pub fn check_in_background<F: Fn(String) + Send + 'static>(
    first_delay: Duration,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_found: F,
) {
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    std::thread::spawn(move || {
        // The first delay exists so a launch at login does not race the
        // network coming up.
        std::thread::sleep(first_delay);
        let mut due = Instant::now();
        loop {
            if enabled.load(Ordering::Relaxed) {
                if Instant::now() >= due {
                    if let Some(version) = check() {
                        on_found(version);
                    }
                    due = Instant::now() + RECHECK;
                }
            } else {
                // Held due while switched off, so turning it on asks straight
                // away instead of serving out the rest of a day nobody was
                // counting.
                due = Instant::now();
            }
            std::thread::sleep(POLL);
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
