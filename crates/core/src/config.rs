//! Reading gcloud's active configuration.
//!
//! All of this comes off disk rather than out of `gcloud config list`. Reading
//! two small INI files takes microseconds; spawning gcloud takes most of a
//! second, and the menu has to be able to redraw itself without stalling.

use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveConfig {
    /// Configuration name, e.g. "default".
    pub name: String,
    pub account: Option<String>,
    pub project: Option<String>,
}

/// Read the currently active configuration.
pub fn active(gcloud_config_dir: &Path) -> Option<ActiveConfig> {
    let name = std::fs::read_to_string(gcloud_config_dir.join("active_config"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());

    let path = gcloud_config_dir
        .join("configurations")
        .join(format!("config_{name}"));
    let body = std::fs::read_to_string(path).ok()?;
    let core = parse_ini_section(&body, "core");
    Some(ActiveConfig {
        name,
        account: core
            .iter()
            .find(|(k, _)| k == "account")
            .map(|(_, v)| v.clone()),
        project: core
            .iter()
            .find(|(k, _)| k == "project")
            .map(|(_, v)| v.clone()),
    })
}

/// Every configuration gcloud knows about, sorted, for the switcher.
pub fn list(gcloud_config_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(gcloud_config_dir.join("configurations")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("config_"))
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// Minimal INI reader for the one shape gcloud writes.
///
/// Deliberately not a general parser: gcloud's files have no quoting, no
/// continuations, and no duplicate sections, and inventing support for things
/// that never appear would only create ways to be wrong.
fn parse_ini_section(body: &str, wanted: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_section = section.trim().eq_ignore_ascii_case(wanted);
            continue;
        }
        if in_section {
            if let Some((k, v)) = line.split_once('=') {
                out.push((k.trim().to_lowercase(), v.trim().to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CONFIG: &str = r#"
[core]
account = nic@glazkov.com
project = my-project
disable_usage_reporting = True

[compute]
region = us-west1
"#;

    #[test]
    fn reads_account_and_project_from_core_only() {
        let got = parse_ini_section(CONFIG, "core");
        assert_eq!(
            got.iter().find(|(k, _)| k == "account").unwrap().1,
            "nic@glazkov.com"
        );
        assert_eq!(
            got.iter().find(|(k, _)| k == "project").unwrap().1,
            "my-project"
        );
        // region belongs to [compute] and must not leak in.
        assert!(got.iter().all(|(k, _)| k != "region"));
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let body = "# a comment\n\n[core]\n; another\nproject = p\n";
        assert_eq!(
            parse_ini_section(body, "core"),
            vec![("project".into(), "p".into())]
        );
    }

    #[test]
    fn reads_a_real_directory_layout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("configurations")).unwrap();
        fs::write(root.join("active_config"), "work\n").unwrap();
        fs::write(root.join("configurations").join("config_work"), CONFIG).unwrap();
        fs::write(
            root.join("configurations").join("config_default"),
            "[core]\n",
        )
        .unwrap();

        let active = active(root).unwrap();
        assert_eq!(active.name, "work");
        assert_eq!(active.account.as_deref(), Some("nic@glazkov.com"));
        assert_eq!(active.project.as_deref(), Some("my-project"));
        assert_eq!(list(root), vec!["default".to_string(), "work".to_string()]);
    }

    #[test]
    fn missing_active_config_file_means_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("configurations")).unwrap();
        fs::write(root.join("configurations").join("config_default"), CONFIG).unwrap();
        assert_eq!(active(root).unwrap().name, "default");
    }

    #[test]
    fn a_configuration_with_no_account_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("configurations")).unwrap();
        fs::write(
            root.join("configurations").join("config_default"),
            "[core]\n",
        )
        .unwrap();
        let a = active(root).unwrap();
        assert!(a.account.is_none() && a.project.is_none());
    }

    #[test]
    fn missing_everything_yields_none() {
        assert!(active(Path::new("/no/such/gcloud")).is_none());
        assert!(list(Path::new("/no/such/gcloud")).is_empty());
    }
}
