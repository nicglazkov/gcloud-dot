//! Reconstructs your login history from gcloud's own invocation logs.
//!
//! gcloud writes one log per command under `logs/YYYY.MM.DD/HH.MM.SS.ffffff.log`.
//! Nothing in there is a documented interface, so this module is deliberately
//! conservative: it looks for two markers rather than one, and it ignores
//! anything it cannot make sense of.

use chrono::{DateTime, Local};
use std::path::Path;
use std::time::SystemTime;

/// Present in the log of any `gcloud auth login` invocation, including the ones
/// that were cancelled at the browser or failed.
const INVOCATION_MARKER: &str = "running [gcloud.auth.login]";

/// Written only once a credential has actually been stored. Requiring one of
/// these is what separates a completed login from an abandoned one.
///
/// The Windows implementation this replaces matched only the invocation marker,
/// so every cancelled login was recorded as a session start. That inflated the
/// login list and poisoned the gap heuristic that runs before real samples
/// exist, because an abandoned attempt followed by a real one two minutes later
/// looked like a two-minute session.
const COMPLETION_MARKERS: &[&str] = &["you are now logged in", "credentials saved to file"];

/// Logs are small. Anything this large is a debug dump, not a login, and
/// reading it would stall the tick loop for no benefit.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// How far back to look. gcloud keeps logs for a month by default; a year of
/// history would not improve an estimator that only keeps 20 samples.
const MAX_DAY_DIRS: usize = 45;

/// Two logins closer together than this are the same login seen twice.
pub const DEDUPE_WINDOW_MINUTES: i64 = 2;

/// Scan for completed logins, newest last.
///
/// `since` skips files untouched since a previous scan, which is what makes the
/// once-a-minute rescan cheap enough to run forever.
pub fn scan_logins(log_dir: &Path, since: Option<SystemTime>) -> Vec<DateTime<Local>> {
    let mut found = Vec::new();
    let Ok(days) = std::fs::read_dir(log_dir) else {
        return found;
    };

    // Day directories are named so that lexical order is chronological order.
    let mut day_paths: Vec<_> = days
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    day_paths.sort();
    let recent = day_paths
        .iter()
        .rev()
        .take(MAX_DAY_DIRS)
        .collect::<Vec<_>>();

    for day in recent {
        let Ok(entries) = std::fs::read_dir(day) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > MAX_LOG_BYTES {
                continue;
            }
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if let Some(cutoff) = since {
                if modified < cutoff {
                    continue;
                }
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if is_completed_login(&content) {
                // The session begins when the login *finishes*, so the file's
                // last-write time is the right timestamp. The filename records
                // when the command started, which on a slow browser hand-off
                // can be minutes earlier.
                found.push(DateTime::<Local>::from(modified));
            }
        }
    }

    found.sort();
    found
}

/// Both markers, or it does not count.
pub fn is_completed_login(log_body: &str) -> bool {
    let lower = log_body.to_lowercase();
    if !lower.contains(INVOCATION_MARKER) {
        return false;
    }
    COMPLETION_MARKERS.iter().any(|m| lower.contains(m))
}

/// Fold newly seen logins into a known list, dropping near-duplicates.
///
/// Returns true when the list changed, which the caller uses to decide whether
/// the countdown needs resetting.
pub fn merge(known: &mut Vec<DateTime<Local>>, incoming: &[DateTime<Local>]) -> bool {
    let mut changed = false;
    for candidate in incoming {
        let duplicate = known
            .iter()
            .any(|k| (*k - *candidate).num_minutes().abs() < DEDUPE_WINDOW_MINUTES);
        if !duplicate {
            known.push(*candidate);
            changed = true;
        }
    }
    if changed {
        known.sort();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::fs;
    use std::io::Write;

    const REAL_LOGIN: &str = r#"
2026-08-07 12:26:51,101 DEBUG    Running [gcloud.auth.login] with arguments: []
2026-08-07 12:26:53,875 INFO     Credentials saved to file: [/Users/nic/.config/gcloud/credentials.db]
2026-08-07 12:26:53,876 INFO     You are now logged in as [nic@glazkov.com].
"#;

    const CANCELLED_LOGIN: &str = r#"
2026-08-07 12:20:01,000 DEBUG    Running [gcloud.auth.login] with arguments: []
2026-08-07 12:20:44,000 ERROR    (gcloud.auth.login) The browser window was closed.
"#;

    const UNRELATED: &str = r#"
2026-08-07 09:00:00,000 DEBUG    Running [gcloud.storage.ls] with arguments: []
"#;

    #[test]
    fn accepts_a_completed_login() {
        assert!(is_completed_login(REAL_LOGIN));
    }

    #[test]
    fn rejects_a_cancelled_login() {
        // The regression that mattered: this file contains the invocation
        // marker, so a one-marker scanner counts it as a session start.
        assert!(!is_completed_login(CANCELLED_LOGIN));
    }

    #[test]
    fn rejects_unrelated_commands() {
        assert!(!is_completed_login(UNRELATED));
    }

    #[test]
    fn rejects_a_completion_line_without_the_invocation() {
        // "You are now logged in" also appears in the docs URL some commands
        // print. Without the invocation marker it means nothing.
        assert!(!is_completed_login("You are now logged in as [x@y.com]."));
    }

    fn write_log(dir: &Path, name: &str, body: &str, modified: SystemTime) {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.set_modified(modified).unwrap();
    }

    #[test]
    fn scans_a_realistic_tree_and_timestamps_by_mtime() {
        let root = tempfile::tempdir().unwrap();
        let logs = root.path();
        let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_785_000_000);

        write_log(
            &logs.join("2026.08.07"),
            "12.26.51.101000.log",
            REAL_LOGIN,
            when,
        );
        write_log(
            &logs.join("2026.08.07"),
            "12.20.01.000000.log",
            CANCELLED_LOGIN,
            when,
        );
        write_log(
            &logs.join("2026.08.06"),
            "09.00.00.000000.log",
            UNRELATED,
            when,
        );

        let found = scan_logins(logs, None);
        assert_eq!(found.len(), 1, "only the completed login should count");
        assert_eq!(found[0], DateTime::<Local>::from(when));
    }

    #[test]
    fn since_filter_skips_old_files() {
        let root = tempfile::tempdir().unwrap();
        let logs = root.path();
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let new = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_785_000_000);

        write_log(
            &logs.join("2026.08.01"),
            "01.00.00.000000.log",
            REAL_LOGIN,
            old,
        );
        write_log(
            &logs.join("2026.08.07"),
            "12.00.00.000000.log",
            REAL_LOGIN,
            new,
        );

        let found = scan_logins(logs, Some(new - std::time::Duration::from_secs(60)));
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        assert!(scan_logins(Path::new("/no/such/place"), None).is_empty());
    }

    #[test]
    fn merge_dedupes_within_the_window() {
        let base = Local::now();
        let mut known = vec![base];
        // Same login, observed a minute apart by two scans.
        assert!(!merge(&mut known, &[base + Duration::seconds(59)]));
        assert_eq!(known.len(), 1);
        // A genuinely different login.
        assert!(merge(&mut known, &[base + Duration::hours(16)]));
        assert_eq!(known.len(), 2);
    }

    #[test]
    fn merge_keeps_the_list_sorted() {
        let base = Local::now();
        let mut known = vec![base];
        merge(
            &mut known,
            &[base - Duration::hours(20), base + Duration::hours(20)],
        );
        let mut sorted = known.clone();
        sorted.sort();
        assert_eq!(known, sorted);
    }
}
