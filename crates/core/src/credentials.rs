//! Credentials other than the interactive login.
//!
//! Application-default credentials are the reason "gcloud works but my code
//! doesn't" is such a common half-hour of lost time: they are a separate
//! credential with a separate lifetime, and nothing in the CLI's output tells
//! you they have gone stale.

use chrono::{DateTime, Local};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdcKind {
    /// Written by `gcloud auth application-default login`. Refreshes against
    /// the same session policy as the user login, so it dies with it.
    UserCredentials,
    /// A downloaded service account key. Does not expire on a session policy;
    /// it lives until someone revokes or rotates it.
    ServiceAccount { client_email: String },
    /// External account, impersonation, or something newer than this app.
    Other { kind: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdcFile {
    pub kind: AdcKind,
    /// Last write time of the credential file. For a service account key this
    /// is the best available proxy for its age: the JSON Google hands you
    /// contains no creation date, and asking the IAM API needs both network
    /// and a permission the user may not have.
    pub file_modified: Option<DateTime<Local>>,
}

#[derive(Deserialize)]
struct RawAdc {
    #[serde(rename = "type")]
    kind: Option<String>,
    client_email: Option<String>,
}

/// Parse whatever is at the ADC path.
pub fn read_adc(path: &Path) -> Option<AdcFile> {
    let body = std::fs::read_to_string(path).ok()?;
    let raw: RawAdc = serde_json::from_str(&body).ok()?;
    let kind = match raw.kind.as_deref() {
        Some("authorized_user") => AdcKind::UserCredentials,
        Some("service_account") => AdcKind::ServiceAccount {
            client_email: raw.client_email.unwrap_or_else(|| "unknown".into()),
        },
        Some(other) => AdcKind::Other {
            kind: other.to_string(),
        },
        None => return None,
    };
    let file_modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(DateTime::<Local>::from);
    Some(AdcFile {
        kind,
        file_modified,
    })
}

/// Age in days of the credential file, if it can be determined.
pub fn age_days(file: &AdcFile, now: DateTime<Local>) -> Option<i64> {
    file.file_modified.map(|m| (now - m).num_days())
}

/// Whether a service account key file is old enough to mention.
///
/// Reported as "key file last written N days ago" rather than "key is N days
/// old", because a re-download resets the mtime without rotating anything. The
/// distinction matters: overstating this would train people to ignore it.
pub fn key_file_is_stale(file: &AdcFile, now: DateTime<Local>, warn_days: i64) -> bool {
    if warn_days <= 0 {
        return false;
    }
    if !matches!(file.kind, AdcKind::ServiceAccount { .. }) {
        return false;
    }
    age_days(file, now).is_some_and(|d| d >= warn_days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::io::Write;

    fn write_json(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adc.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        (dir, path)
    }

    #[test]
    fn recognises_user_credentials() {
        let (_d, p) = write_json(
            r#"{"client_id":"x.apps.googleusercontent.com","refresh_token":"1//y","type":"authorized_user"}"#,
        );
        assert_eq!(read_adc(&p).unwrap().kind, AdcKind::UserCredentials);
    }

    #[test]
    fn recognises_a_service_account_key() {
        let (_d, p) = write_json(
            r#"{"type":"service_account","project_id":"p","client_email":"sa@p.iam.gserviceaccount.com","private_key_id":"abc"}"#,
        );
        assert_eq!(
            read_adc(&p).unwrap().kind,
            AdcKind::ServiceAccount {
                client_email: "sa@p.iam.gserviceaccount.com".into()
            }
        );
    }

    #[test]
    fn recognises_an_unfamiliar_type_without_failing() {
        let (_d, p) = write_json(r#"{"type":"external_account"}"#);
        assert_eq!(
            read_adc(&p).unwrap().kind,
            AdcKind::Other {
                kind: "external_account".into()
            }
        );
    }

    #[test]
    fn rejects_json_that_is_not_a_credential() {
        let (_d, p) = write_json(r#"{"hello":"world"}"#);
        assert!(read_adc(&p).is_none());
    }

    #[test]
    fn missing_file_is_not_an_error() {
        assert!(read_adc(Path::new("/no/such/adc.json")).is_none());
    }

    #[test]
    fn only_service_account_keys_go_stale() {
        let now = Local::now();
        let old = Some(now - Duration::days(200));
        let user = AdcFile {
            kind: AdcKind::UserCredentials,
            file_modified: old,
        };
        // A user ADC being old means nothing; it refreshes on the session.
        assert!(!key_file_is_stale(&user, now, 90));

        let sa = AdcFile {
            kind: AdcKind::ServiceAccount {
                client_email: "sa@p.iam.gserviceaccount.com".into(),
            },
            file_modified: old,
        };
        assert!(key_file_is_stale(&sa, now, 90));
        assert!(!key_file_is_stale(&sa, now, 0), "zero disables the warning");
        assert!(!key_file_is_stale(&sa, now, 365));
    }
}
