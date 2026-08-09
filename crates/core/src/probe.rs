//! The ground-truth half of the app.
//!
//! Everything else in this crate is inference. This module runs a real command
//! and reports what actually happened, which is why the UI is allowed to state
//! its result as fact.

use std::process::Stdio;
use std::time::Duration;

/// What a probe actually established.
///
/// The third variant is the one that keeps the app trustworthy. A probe that
/// fails for a reason we do not recognise is *not* evidence of expiry, it is
/// usually a dropped network, so it must not be allowed to turn the icon red.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// A token came back. The credential is alive right now.
    Valid,
    /// Google explicitly refused: the session is over and a login is required.
    Expired { detail: String },
    /// Something else went wrong. Hold the previous verdict.
    Indeterminate { detail: String },
}

/// Substrings that mean "this credential is finished", as opposed to "the call
/// did not go through".
///
/// Collected from both the macOS and Windows implementations plus the errors
/// gcloud emits for a revoked refresh token. Matched case-insensitively against
/// stderr.
const REAUTH_MARKERS: &[&str] = &[
    "reauthentication required",
    "reauthentication failed",
    "invalid_rapt",
    "invalid_grant",
    "no credentialed accounts",
    "do not currently have an active account",
    "credentials were revoked",
    "token has been expired or revoked",
    "account has been disabled",
    "please run:\n\n  $ gcloud auth login",
];

/// Classify the result of an auth command.
///
/// Split out from process spawning so the decision table is testable without
/// gcloud installed, which is the whole reason it lives in its own function.
pub fn classify(exit_ok: bool, stdout: &str, stderr: &str) -> ProbeOutcome {
    if exit_ok && !stdout.trim().is_empty() {
        return ProbeOutcome::Valid;
    }
    let haystack = stderr.to_lowercase();
    for marker in REAUTH_MARKERS {
        if haystack.contains(&marker.to_lowercase()) {
            return ProbeOutcome::Expired {
                detail: first_meaningful_line(stderr),
            };
        }
    }
    // Exit 0 with empty stdout is not success; it means gcloud produced no
    // token and said nothing useful about why.
    ProbeOutcome::Indeterminate {
        detail: if stderr.trim().is_empty() {
            "no output from gcloud".to_string()
        } else {
            first_meaningful_line(stderr)
        },
    }
}

/// gcloud prefixes most failures with a blank line and a bare "ERROR:" banner;
/// the sentence after it is the part worth showing a human.
fn first_meaningful_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.eq_ignore_ascii_case("error:"))
        .unwrap_or("")
        .trim_start_matches("ERROR:")
        .trim();
    let mut out: String = line.chars().take(200).collect();
    if line.chars().count() > 200 {
        out.push_str(" (truncated)");
    }
    out
}

/// Which credential to test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    /// The user login established by `gcloud auth login`.
    User,
    /// Application-default credentials, used by client libraries rather than
    /// by the CLI. These expire on their own schedule, which is why local dev
    /// can break while `gcloud` itself still works perfectly.
    ApplicationDefault,
}

impl Credential {
    fn args(self) -> &'static [&'static str] {
        match self {
            Credential::User => &["auth", "print-access-token"],
            Credential::ApplicationDefault => {
                &["auth", "application-default", "print-access-token"]
            }
        }
    }
}

/// Run the probe.
///
/// Refreshing an access token does **not** extend the reauth session: the
/// session length is fixed when the session is created. That is what makes it
/// safe to poll every few minutes without corrupting the very measurement the
/// estimator is taking.
pub fn run(gcloud: &std::path::Path, which: Credential, timeout: Duration) -> ProbeOutcome {
    let mut cmd = crate::proc::quiet(gcloud);
    cmd.args(which.args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Without this gcloud can block forever waiting on a y/n it will never
        // get, since we have given it no stdin to read from.
        .env("CLOUDSDK_CORE_DISABLE_PROMPTS", "1");

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ProbeOutcome::Indeterminate {
                detail: format!("could not run gcloud: {e}"),
            }
        }
    };

    match wait_with_timeout(child, timeout) {
        Some(out) => classify(
            out.status.success(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        None => ProbeOutcome::Indeterminate {
            detail: "gcloud timed out".to_string(),
        },
    }
}

/// `std::process` has no timed wait, and a gcloud wedged on a captive-portal
/// network will otherwise hang the tick loop forever. Polling at 50 ms costs
/// nothing next to a network round trip.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Option<std::process::Output> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_on_stdout_is_valid() {
        assert_eq!(
            classify(true, "ya29.a0AfB_byExample", ""),
            ProbeOutcome::Valid
        );
    }

    #[test]
    fn success_with_no_token_is_not_valid() {
        // Exit 0 but nothing to show for it: report unknown rather than claim
        // a credential we never actually saw.
        assert!(matches!(
            classify(true, "   \n", ""),
            ProbeOutcome::Indeterminate { .. }
        ));
    }

    #[test]
    fn recognises_every_reauth_marker() {
        let cases = [
            "ERROR: (gcloud.auth.print-access-token) There was a problem refreshing your current auth tokens: invalid_grant",
            "ERROR: Reauthentication required.",
            "invalid_rapt: reauth required",
            "ERROR: (gcloud.auth.print-access-token) You do not currently have an active account selected.",
            "Token has been expired or revoked.",
        ];
        for stderr in cases {
            assert!(
                matches!(classify(false, "", stderr), ProbeOutcome::Expired { .. }),
                "should have been read as expired: {stderr}"
            );
        }
    }

    #[test]
    fn network_failure_is_never_expiry() {
        // The distinction the whole design rests on. A red dot must mean the
        // session is gone, not that the coffee shop wifi dropped.
        let stderr = "ERROR: (gcloud.auth.print-access-token) There was a problem \
                      refreshing your current auth tokens: Unable to find the server at \
                      oauth2.googleapis.com";
        assert!(matches!(
            classify(false, "", stderr),
            ProbeOutcome::Indeterminate { .. }
        ));
    }

    #[test]
    fn matching_ignores_case() {
        assert!(matches!(
            classify(false, "", "INVALID_GRANT"),
            ProbeOutcome::Expired { .. }
        ));
    }

    #[test]
    fn detail_skips_the_bare_error_banner() {
        let stderr = "\nERROR:\ninvalid_grant: token expired\n";
        match classify(false, "", stderr) {
            ProbeOutcome::Expired { detail } => assert_eq!(detail, "invalid_grant: token expired"),
            other => panic!("expected expiry, got {other:?}"),
        }
    }

    #[test]
    fn detail_is_bounded() {
        let stderr = format!("invalid_grant: {}", "x".repeat(500));
        match classify(false, "", &stderr) {
            ProbeOutcome::Expired { detail } => {
                assert!(detail.chars().count() <= 212);
                assert!(detail.ends_with(" (truncated)"));
            }
            other => panic!("expected expiry, got {other:?}"),
        }
    }
}
