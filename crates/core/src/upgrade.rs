//! Finding, fetching, and applying a new version.
//!
//! The hard part is not downloading a file. It is knowing whether this copy of
//! the app is allowed to replace itself at all.
//!
//! When a package manager installed it, that manager owns the files and keeps a
//! database describing them. Overwriting them behind its back leaves it
//! describing a version that is no longer on disk, and the next upgrade it runs
//! will happily put the old one back. So the updater works out how it was
//! installed and does the right thing for that route: replace itself only when
//! nothing else is managing it, run the manager when running it needs no
//! elevation, and otherwise say the command and stay out of the way.

use crate::proc;
use std::path::{Path, PathBuf};

const REPO: &str = "nicglazkov/gcloud-dot";
pub const RELEASES_PAGE: &str = "https://github.com/nicglazkov/gcloud-dot/releases/latest";

/// How this copy of the app arrived on the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Nothing is managing it: a disk image dragged to Applications, the shell
    /// installer, or a binary someone put somewhere. Safe to replace in place.
    SelfManaged,
    /// Homebrew, which can be upgraded without elevation.
    Homebrew,
    /// Installed on Windows by this project's own installer, whether the user
    /// ran it directly or winget ran it for them.
    ///
    /// Those two are deliberately one case. Both write the same registry keys,
    /// so nothing on disk tells them apart, and the answer is the same either
    /// way: run that installer again. It replaces the files and rewrites the
    /// Add or remove programs entry, which is where winget reads the installed
    /// version from, so both routes stay truthful. Running `winget upgrade`
    /// instead would fail outright for the many users who never used winget.
    WindowsInstaller,
    /// dpkg or apt. Upgrading needs root, so the app will not attempt it.
    DebPackage,
    /// An Arch package. Upgrading needs root.
    ArchPackage,
    /// An AppImage, which is a single file the user placed themselves. It can
    /// be replaced, but only the user knows where it should live.
    AppImage,
}

impl InstallKind {
    /// Can the app put the new version in place itself?
    pub fn can_self_replace(self) -> bool {
        matches!(
            self,
            InstallKind::SelfManaged | InstallKind::AppImage | InstallKind::WindowsInstaller
        )
    }

    /// Can the app run the upgrade for the user without asking for a password?
    pub fn can_run_manager(self) -> bool {
        matches!(self, InstallKind::Homebrew)
    }

    /// The command that upgrades this install, for showing or for running.
    pub fn manager_command(self) -> Option<&'static str> {
        match self {
            InstallKind::Homebrew => Some("brew upgrade --cask nicglazkov/tap/gcloud-dot"),
            InstallKind::DebPackage => Some("sudo apt install ./gcloud-dot_<version>_<arch>.deb"),
            InstallKind::ArchPackage => Some("yay -Syu gcloud-dot"),
            InstallKind::SelfManaged | InstallKind::AppImage | InstallKind::WindowsInstaller => {
                None
            }
        }
    }

    /// What to tell someone who cannot be upgraded from inside the app.
    pub fn why_manual(self) -> Option<&'static str> {
        match self {
            InstallKind::DebPackage => Some(
                "This copy was installed by apt, which owns the files and needs root to \
                 replace them.",
            ),
            InstallKind::ArchPackage => Some(
                "This copy was installed by an Arch package, which owns the files and needs \
                 root to replace them.",
            ),
            _ => None,
        }
    }
}

/// The facts about the filesystem that decide the answer.
///
/// Separated from the code that gathers them so the decision itself can be
/// tested without installing four package managers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Evidence {
    pub in_homebrew_caskroom: bool,
    pub has_dpkg_record: bool,
    pub has_pacman_record: bool,
    pub has_windows_install_record: bool,
    pub is_appimage: bool,
}

/// Decide from the executable's location and the surrounding evidence.
pub fn classify(exe: &Path, e: Evidence) -> InstallKind {
    if e.is_appimage {
        return InstallKind::AppImage;
    }
    if e.in_homebrew_caskroom {
        return InstallKind::Homebrew;
    }
    if e.has_dpkg_record {
        return InstallKind::DebPackage;
    }
    if e.has_pacman_record {
        return InstallKind::ArchPackage;
    }
    if e.has_windows_install_record {
        return InstallKind::WindowsInstaller;
    }
    // A binary under a system prefix that no package manager claims is still
    // not ours to overwrite: it needs root, and something put it there.
    let p = exe.to_string_lossy();
    let under_system_prefix =
        p.starts_with("/usr/") || (p.starts_with("/opt/") && !p.contains("homebrew"));
    if under_system_prefix {
        return InstallKind::DebPackage;
    }
    InstallKind::SelfManaged
}

/// Gather the evidence and classify this running copy.
///
/// Every question below is asked about *this executable*, not about the machine.
/// "A cask is installed somewhere" is not the same claim as "this file belongs
/// to that cask", and answering the easier question would tell a developer
/// running a build out of `target/debug` that Homebrew owns it.
pub fn detect() -> InstallKind {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut e = Evidence {
        is_appimage: std::env::var_os("APPIMAGE").is_some(),
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    {
        // The cask puts the bundle in /Applications and records it in the
        // Caskroom. Both have to be true: the Caskroom alone would also match a
        // build running from anywhere else on the same machine.
        let is_the_installed_bundle = exe.starts_with("/Applications/GCloud Dot.app");
        e.in_homebrew_caskroom = is_the_installed_bundle
            && ["/opt/homebrew", "/usr/local"]
                .iter()
                .any(|p| Path::new(p).join("Caskroom/gcloud-dot").is_dir());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // dpkg lists the files it owns, so this can be asked precisely.
        e.has_dpkg_record = std::fs::read_to_string("/var/lib/dpkg/info/gcloud-dot.list")
            .map(|list| list.lines().any(|f| Path::new(f.trim()) == exe))
            .unwrap_or(false);
        e.has_pacman_record = std::fs::read_dir("/var/lib/pacman/local")
            .map(|d| {
                d.flatten()
                    .any(|x| x.file_name().to_string_lossy().starts_with("gcloud-dot-"))
            })
            .unwrap_or(false)
            && exe.starts_with("/usr/");
    }

    #[cfg(windows)]
    {
        // The installer records where it put the files, and winget leaves the
        // same record because it runs that same installer. One case, one answer.
        e.has_windows_install_record = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey(r"Software\GCloudDot")
            .and_then(|k| k.get_value::<String, _>("InstallDir"))
            .map(|dir| exe.starts_with(&dir))
            .unwrap_or(false);
    }

    classify(&exe, e)
}

/// A release worth offering.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

impl Release {
    /// The asset this platform should download, if there is one.
    pub fn asset_for_this_platform(&self, kind: InstallKind) -> Option<&Asset> {
        let want = platform_asset_suffix(kind);
        self.assets.iter().find(|a| a.name.ends_with(want))
    }

    /// The published checksums, which every release carries.
    pub fn checksums_asset(&self) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == "SHA256SUMS.txt")
    }
}

/// What to fetch for this platform and install shape.
///
/// The architecture is part of every answer, including the AppImage one. Only
/// an x86_64 AppImage is published, and a bare `.AppImage` suffix would hand it
/// to someone on arm64, where it would install cleanly and then refuse to run.
/// Matching nothing is the better failure: it produces "this release has no
/// download for this platform" and leaves the working copy alone.
pub fn platform_asset_suffix(kind: InstallKind) -> &'static str {
    if kind == InstallKind::AppImage {
        return if cfg!(target_arch = "aarch64") {
            "-aarch64.AppImage"
        } else {
            "-x86_64.AppImage"
        };
    }
    if cfg!(target_os = "macos") {
        ".dmg"
    } else if cfg!(windows) {
        "-setup.exe"
    } else if cfg!(target_arch = "aarch64") {
        "gcloud-dot-linux-aarch64.tar.gz"
    } else {
        "gcloud-dot-linux-x86_64.tar.gz"
    }
}

/// Ask GitHub what the newest release is.
///
/// `curl` rather than a linked HTTP client: this is one request a day, and a
/// TLS stack would add more to the binary than the rest of the app together.
pub fn latest() -> Option<Release> {
    let out = proc::quiet("curl")
        .args([
            "-fsSL",
            "--max-time",
            "20",
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            concat!("gcloud-dot/", env!("CARGO_PKG_VERSION")),
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_release(&String::from_utf8_lossy(&out.stdout))
}

/// Pull the version and assets out of GitHub's release document.
pub fn parse_release(body: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let version = v
        .get("tag_name")?
        .as_str()?
        .trim_start_matches('v')
        .to_string();
    let assets = v
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(|a| {
            Some(Asset {
                name: a.get("name")?.as_str()?.to_string(),
                url: a.get("browser_download_url")?.as_str()?.to_string(),
            })
        })
        .collect();
    Some(Release { version, assets })
}

/// Numeric comparison of dotted versions.
///
/// Anything non numeric compares as zero, so a pre-release sorts below the
/// release it precedes. That is the safe direction: the alternative is telling
/// every user of a stable build to move to a beta.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split(['.', '-', '+'])
            .map(|p| p.parse().unwrap_or(0))
            .take(4)
            .collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Look up one file's expected hash in a published `SHA256SUMS.txt`.
pub fn expected_hash(checksums: &str, name: &str) -> Option<String> {
    for line in checksums.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (hash, file) = line.split_once(char::is_whitespace)?;
        if file.trim().trim_start_matches("./") == name {
            return Some(hash.trim().to_lowercase());
        }
    }
    None
}

/// Download a URL to a path, following redirects, failing on any HTTP error.
pub fn download(url: &str, to: &Path) -> Result<(), String> {
    let out = proc::quiet("curl")
        .args(["-fsSL", "--max-time", "300", "-o"])
        .arg(to)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "download failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// The SHA-256 of a file, as lowercase hex.
///
/// Shelling out to the system tool rather than linking a hash implementation,
/// for the same reason as `curl`: it runs twice a release and every supported
/// system already has it.
pub fn sha256_of(path: &Path) -> Option<String> {
    #[cfg(windows)]
    let out = proc::quiet("certutil")
        .arg("-hashfile")
        .arg(path)
        .arg("SHA256")
        .output()
        .ok()?;
    #[cfg(not(windows))]
    let out = proc::quiet("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&out.stdout);
    #[cfg(windows)]
    {
        // certutil prints a banner, the hash, then a completion line.
        text.lines()
            .map(str::trim)
            .find(|l| l.len() == 64 && l.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|l| l.to_lowercase())
    }
    #[cfg(not(windows))]
    {
        text.split_whitespace().next().map(|h| h.to_lowercase())
    }
}

/// Where downloads are staged. Beside the state file, so it is on the same
/// filesystem as nothing in particular but is ours to clean up.
pub fn staging_dir() -> PathBuf {
    crate::paths::data_dir().join("update")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_managed_copy_never_replaces_itself() {
        let brew = classify(
            Path::new("/Applications/GCloud Dot.app/Contents/MacOS/GCloudDot"),
            Evidence {
                in_homebrew_caskroom: true,
                ..Default::default()
            },
        );
        assert_eq!(brew, InstallKind::Homebrew);
        assert!(!brew.can_self_replace());
        assert!(brew.can_run_manager(), "brew needs no password");

        let win = classify(
            Path::new(r"C:\Users\live\AppData\Local\Programs\GCloud Dot\gcloud-dot-tray.exe"),
            Evidence {
                has_windows_install_record: true,
                ..Default::default()
            },
        );
        assert_eq!(win, InstallKind::WindowsInstaller);
        assert!(
            win.can_self_replace(),
            "running our own installer again is the correct route for both \
             the direct download and winget, and the only one that works when \
             winget has never heard of this package"
        );
        assert!(!win.can_run_manager());
        assert!(win.manager_command().is_none());

        let deb = classify(
            Path::new("/usr/bin/gcloud-dot-tray"),
            Evidence {
                has_dpkg_record: true,
                ..Default::default()
            },
        );
        assert_eq!(deb, InstallKind::DebPackage);
        assert!(!deb.can_self_replace());
        assert!(!deb.can_run_manager(), "apt needs root, so do not try");
        assert!(deb.why_manual().is_some());
    }

    #[test]
    fn a_dragged_app_manages_itself() {
        let k = classify(
            Path::new("/Applications/GCloud Dot.app/Contents/MacOS/GCloudDot"),
            Evidence::default(),
        );
        assert_eq!(k, InstallKind::SelfManaged);
        assert!(k.can_self_replace());
        assert!(k.manager_command().is_none());
    }

    #[test]
    fn a_script_install_manages_itself() {
        let k = classify(
            Path::new("/Users/nic/.local/bin/gcloud-dot-tray"),
            Evidence::default(),
        );
        assert_eq!(k, InstallKind::SelfManaged);
        assert!(k.can_self_replace());
    }

    #[test]
    fn a_system_prefix_is_never_assumed_to_be_ours() {
        // Nothing claims it, but it still took root to put there, so replacing
        // it would need root too.
        let k = classify(
            Path::new("/usr/local/bin/gcloud-dot-tray"),
            Evidence::default(),
        );
        assert!(!k.can_self_replace(), "{k:?} should not self replace");
    }

    #[test]
    fn an_appimage_is_a_single_file_we_can_swap() {
        let k = classify(
            Path::new("/home/nic/Apps/GCloud_Dot.AppImage"),
            Evidence {
                is_appimage: true,
                ..Default::default()
            },
        );
        assert_eq!(k, InstallKind::AppImage);
        assert!(k.can_self_replace());
        assert!(platform_asset_suffix(k).ends_with(".AppImage"));
    }

    #[test]
    fn a_development_build_is_never_treated_as_managed() {
        // The machine this is written on has the cask installed. A build run
        // out of the target directory is still not the cask's copy, and telling
        // it otherwise would send every developer to `brew upgrade`.
        let k = detect();
        let exe = std::env::current_exe().unwrap();
        if exe.to_string_lossy().contains("/target/") {
            assert_eq!(
                k,
                InstallKind::SelfManaged,
                "{} classified as {k:?}",
                exe.display()
            );
        }
    }

    #[test]
    fn detects_newer_versions_only() {
        assert!(is_newer("1.0.6", "1.0.5"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(!is_newer("1.0.5", "1.0.5"));
        assert!(!is_newer("1.0.4", "1.0.5"));
        assert!(!is_newer("1.0.5-rc1", "1.0.5"), "a beta must not prompt");
        assert!(!is_newer("nightly", "1.0.5"));
    }

    const RELEASE_JSON: &str = r#"{
        "tag_name": "v1.0.6",
        "assets": [
          {"name": "GCloud-Dot-1.0.6.dmg", "browser_download_url": "https://example.test/a.dmg"},
          {"name": "GCloud-Dot-1.0.6-setup.exe", "browser_download_url": "https://example.test/b.exe"},
          {"name": "gcloud-dot-linux-x86_64.tar.gz", "browser_download_url": "https://example.test/c.tgz"},
          {"name": "gcloud-dot-linux-aarch64.tar.gz", "browser_download_url": "https://example.test/d.tgz"},
          {"name": "GCloud_Dot-1.0.6-x86_64.AppImage", "browser_download_url": "https://example.test/e.AppImage"},
          {"name": "SHA256SUMS.txt", "browser_download_url": "https://example.test/sums"}
        ]
    }"#;

    #[test]
    fn reads_a_real_release_document() {
        let r = parse_release(RELEASE_JSON).unwrap();
        assert_eq!(r.version, "1.0.6");
        assert_eq!(r.assets.len(), 6);
        assert!(r.checksums_asset().is_some());
    }

    #[test]
    fn picks_the_asset_for_this_platform() {
        let r = parse_release(RELEASE_JSON).unwrap();
        let a = r.asset_for_this_platform(InstallKind::SelfManaged).unwrap();
        if cfg!(target_os = "macos") {
            assert!(a.name.ends_with(".dmg"));
        } else if cfg!(windows) {
            assert!(a.name.ends_with("-setup.exe"));
        } else {
            assert!(a.name.ends_with(".tar.gz"));
        }
        // An AppImage install takes the AppImage whatever the platform
        // default, but only one built for this architecture.
        let img = r.asset_for_this_platform(InstallKind::AppImage);
        if cfg!(target_arch = "aarch64") {
            assert!(
                img.is_none(),
                "no arm64 AppImage is published, so offer nothing"
            );
        } else {
            assert!(img.unwrap().name.ends_with("-x86_64.AppImage"));
        }
    }

    #[test]
    fn a_release_missing_our_asset_offers_nothing() {
        let r = Release {
            version: "9.9.9".into(),
            assets: vec![Asset {
                name: "something-else.txt".into(),
                url: "https://example.test/x".into(),
            }],
        };
        assert!(r
            .asset_for_this_platform(InstallKind::SelfManaged)
            .is_none());
    }

    #[test]
    fn finds_a_hash_in_the_published_checksums() {
        let sums = "abc123  ./GCloud-Dot-1.0.6.dmg\ndef456  gcloud-dot-windows-x86_64.zip\n";
        assert_eq!(
            expected_hash(sums, "GCloud-Dot-1.0.6.dmg").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            expected_hash(sums, "gcloud-dot-windows-x86_64.zip").as_deref(),
            Some("def456")
        );
        assert!(expected_hash(sums, "not-there.dmg").is_none());
    }

    #[test]
    fn hashes_compare_without_case_tripping_them_up() {
        // certutil prints uppercase on Windows, shasum lowercase everywhere else.
        let sums = "ABCDEF  file.dmg\n";
        assert_eq!(expected_hash(sums, "file.dmg").as_deref(), Some("abcdef"));
    }
}

// ---------------------------------------------------------------- applying

/// Our Developer ID team. A downloaded macOS app is only trusted if it is
/// signed by this team and accepted by Gatekeeper.
#[cfg(target_os = "macos")]
const TEAM_ID: &str = "M7D6YHVDNK";

/// Download the new version, check it, and put it in place.
///
/// `progress` is called with short lines suitable for showing in a window.
pub fn apply(release: &Release, kind: InstallKind, progress: &dyn Fn(&str)) -> Result<(), String> {
    if !kind.can_self_replace() {
        return Err("this copy is managed by a package manager".into());
    }
    let asset = release
        .asset_for_this_platform(kind)
        .ok_or("this release has no download for this platform")?;
    let sums = release
        .checksums_asset()
        .ok_or("this release publishes no checksums, so it cannot be verified")?;

    let dir = staging_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not stage the download: {e}"))?;

    progress("Downloading");
    let payload = dir.join(&asset.name);
    download(&asset.url, &payload)?;

    progress("Checking the download");
    let sums_path = dir.join("SHA256SUMS.txt");
    download(&sums.url, &sums_path)?;
    let published = std::fs::read_to_string(&sums_path)
        .ok()
        .and_then(|s| expected_hash(&s, &asset.name))
        .ok_or("the release does not publish a checksum for this file")?;
    let actual = sha256_of(&payload).ok_or("could not hash the download")?;
    if actual != published {
        return Err("the download does not match its published checksum".into());
    }

    progress("Installing");
    let result = install_payload(&payload, kind);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(target_os = "macos")]
fn install_payload(dmg: &Path, _kind: InstallKind) -> Result<(), String> {
    // Mounted at a path of our choosing. Some environments will not let a
    // process create a mount point under /Volumes, and a private mount point
    // avoids colliding with a volume of the same name already attached.
    let mount = staging_dir().join("mnt");
    std::fs::create_dir_all(&mount).map_err(|e| format!("{e}"))?;
    let out = proc::quiet("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-noautoopen",
            "-readonly",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(dmg)
        .output()
        .map_err(|e| format!("could not mount the download: {e}"))?;
    if !out.status.success() {
        return Err("could not mount the download".into());
    }
    let detach = || {
        let _ = proc::quiet("hdiutil").arg("detach").arg(&mount).output();
    };

    let new_app = mount.join("GCloud Dot.app");
    if !new_app.is_dir() {
        detach();
        return Err("the disk image does not contain the app".into());
    }

    // Verify before trusting it. The checksum only proves the bytes match what
    // the release page published; the signature proves who built them.
    if let Err(e) = verify_signed_by_us(&new_app) {
        detach();
        return Err(e);
    }

    let target = current_app_bundle().ok_or_else(|| {
        detach();
        "could not work out where this app is installed".to_string()
    })?;

    // Copy out of the read only image first, then swap. ditto preserves the
    // signature, which cp -R does not always do for a bundle.
    let staged = staging_dir().join("GCloud Dot.app");
    let _ = std::fs::remove_dir_all(&staged);
    let copy = proc::quiet("ditto").arg(&new_app).arg(&staged).output();
    detach();
    match copy {
        Ok(o) if o.status.success() => {}
        _ => return Err("could not copy the new app out of the disk image".into()),
    }

    // Swap: move the old one aside, move the new one in, then delete the old.
    // Renaming rather than deleting first means a failure leaves a working app
    // rather than none at all.
    let aside = target.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&aside);
    std::fs::rename(&target, &aside).map_err(|e| format!("could not move the old app: {e}"))?;
    if let Err(e) = std::fs::rename(&staged, &target) {
        let _ = std::fs::rename(&aside, &target);
        return Err(format!("could not put the new app in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
}

/// Confirm a downloaded bundle is ours and that Gatekeeper accepts it.
#[cfg(target_os = "macos")]
fn verify_signed_by_us(app: &Path) -> Result<(), String> {
    let info = proc::quiet("codesign")
        .args(["-dv", "--verbose=4"])
        .arg(app)
        .output()
        .map_err(|e| format!("could not check the signature: {e}"))?;
    // codesign writes its report to stderr.
    let text = String::from_utf8_lossy(&info.stderr).to_string();
    if !text.contains(&format!("TeamIdentifier={TEAM_ID}")) {
        return Err("the downloaded app is not signed by this project".into());
    }
    let verified = proc::quiet("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .output()
        .map_err(|e| format!("could not verify the signature: {e}"))?;
    if !verified.status.success() {
        return Err("the downloaded app's signature is not valid".into());
    }
    // Gatekeeper's own verdict, which is what the user would face on launch.
    let assessed = proc::quiet("spctl")
        .args(["-a", "-t", "exec"])
        .arg(app)
        .output()
        .map_err(|e| format!("could not assess the download: {e}"))?;
    if !assessed.status.success() {
        return Err("the downloaded app is not notarized".into());
    }
    Ok(())
}

/// The `.app` this executable lives inside, if it does.
#[cfg(target_os = "macos")]
pub fn current_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // .../GCloud Dot.app/Contents/MacOS/GCloudDot
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()?.to_str()? == "app").then(|| bundle.to_path_buf())
}

#[cfg(windows)]
fn install_payload(setup: &Path, _kind: InstallKind) -> Result<(), String> {
    // Windows will not overwrite an image that is running, and the process
    // asking for this upgrade is running from one of the two files the
    // installer is about to replace. The installer stops the tray before it
    // writes, which covers that one, but nothing can stop the process doing
    // the asking.
    //
    // Renaming is allowed where overwriting is not: an open handle follows the
    // file rather than the path, so this process carries on executing from the
    // renamed copy and the installer finds the original name free. Without
    // this, `gcloud-dot upgrade` reported success while leaving the command
    // line at the old version, because every File command in the installer had
    // quietly failed to write over the running executable.
    let displaced = std::env::current_exe().ok().and_then(|exe| {
        let aside = exe.with_extension("exe.old");
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(&exe, &aside).ok().map(|_| (exe, aside))
    });

    let status = proc::quiet(setup)
        .arg("/S")
        .status()
        .map_err(|e| format!("could not run the installer: {e}"))?;

    if !status.success() {
        // Put it back. A failed upgrade must not also uninstall the thing it
        // failed to replace.
        if let Some((exe, aside)) = &displaced {
            if !exe.exists() {
                let _ = std::fs::rename(aside, exe);
            }
        }
        return Err("the installer did not finish".into());
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install_payload(payload: &Path, kind: InstallKind) -> Result<(), String> {
    if kind == InstallKind::AppImage {
        let target = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .ok_or("could not work out where this AppImage lives")?;
        return replace_file(payload, &target);
    }

    // A tarball of the two binaries, beside whatever is running now.
    let dir = staging_dir().join("unpacked");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{e}"))?;
    let out = proc::quiet("tar")
        .arg("-xzf")
        .arg(payload)
        .arg("-C")
        .arg(&dir)
        .output()
        .map_err(|e| format!("could not unpack the download: {e}"))?;
    if !out.status.success() {
        return Err("could not unpack the download".into());
    }

    let here = std::env::current_exe()
        .map_err(|e| format!("{e}"))?
        .parent()
        .ok_or("no install directory")?
        .to_path_buf();
    for name in ["gcloud-dot", "gcloud-dot-tray"] {
        let src = dir.join(name);
        if src.is_file() {
            replace_file(&src, &here.join(name))?;
        }
    }
    Ok(())
}

/// Put a file in place without writing into the one that is running.
///
/// Writing into a running executable fails on Linux with ETXTBSY and corrupts
/// the process on macOS. Renaming over it leaves the old inode alone, so
/// anything still executing finishes on the old code and the new file is what
/// starts next.
#[cfg(all(unix, not(target_os = "macos")))]
fn replace_file(src: &Path, dst: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let staged = dst.with_extension("new");
    std::fs::copy(src, &staged).map_err(|e| format!("could not stage {}: {e}", dst.display()))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("could not make {} executable: {e}", staged.display()))?;
    std::fs::rename(&staged, dst).map_err(|e| format!("could not replace {}: {e}", dst.display()))
}

// ------------------------------------------------------------- orchestration

/// What an upgrade attempt did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Already newest. Carries the version in hand.
    UpToDate(String),
    /// Replaced or handed to a package manager that has finished.
    Upgraded { from: String, to: String },
    /// Started a package manager that will finish on its own.
    Handed { to: String, command: String },
    /// Cannot be done from here. Carries the command the user should run.
    Manual {
        to: String,
        command: String,
        why: String,
    },
}

/// The whole upgrade, from asking GitHub to putting files in place.
///
/// Shared by the command line and the window so the two cannot drift. The
/// caller supplies `progress` to show what is happening and decides what to do
/// about a restart afterwards.
pub fn run(current: &str, progress: &dyn Fn(&str)) -> Result<Outcome, String> {
    progress("Checking for a new version");
    let release = latest().ok_or("could not reach GitHub to check for a new version")?;
    if !is_newer(&release.version, current) {
        return Ok(Outcome::UpToDate(current.to_string()));
    }

    let kind = detect();
    let to = release.version.clone();

    if kind.can_self_replace() {
        apply(&release, kind, progress)?;
        return Ok(Outcome::Upgraded {
            from: current.to_string(),
            to,
        });
    }

    let command = kind
        .manager_command()
        .ok_or("this copy cannot be upgraded from here")?
        .to_string();

    if kind.can_run_manager() {
        progress("Handing over to the package manager");
        start_manager(kind)?;
        return Ok(Outcome::Handed { to, command });
    }

    Ok(Outcome::Manual {
        to,
        command,
        why: kind.why_manual().unwrap_or_default().to_string(),
    })
}

/// Start the package manager and leave it running.
///
/// Not waited on. Homebrew stops the app as part of upgrading a cask, so the
/// process that started it is often gone before it finishes; treating that as
/// a failure would report one on every successful upgrade.
fn start_manager(kind: InstallKind) -> Result<(), String> {
    let mut cmd = match kind {
        InstallKind::Homebrew => {
            let mut c = proc::quiet("brew");
            c.args(["upgrade", "--cask", "nicglazkov/tap/gcloud-dot"]);
            c
        }
        _ => return Err("no package manager to run".into()),
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the package manager: {e}"))
}

/// Remove an executable that a previous upgrade renamed out of the way.
///
/// Windows cannot delete a running image, so the upgrade renames the file it is
/// running from and lets the installer clear the leftover. The installer only
/// gets one attempt though, and at that moment the process holding the file is
/// usually still alive, so the delete is deferred to the next reboot. This runs
/// at startup, when nothing holds it, and closes that gap for anyone who does
/// not reboot.
///
/// Best effort throughout. A file that cannot be removed will be caught by the
/// next upgrade, or by the reboot already scheduled for it.
pub fn clear_replaced_files() {
    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(dir) = exe.parent() else {
            return;
        };
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "old") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

/// Arrange for a fresh copy to start shortly, then let the caller exit.
///
/// The delay matters. The replacement is already on disk, so the new copy could
/// start at once, but the old one is still holding its tray icon and its single
/// instance marker. Letting it finish exiting first avoids two icons appearing
/// side by side for as long as the handover takes.
pub fn schedule_relaunch() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle = current_app_bundle().ok_or("could not find the app to restart")?;
        let script = format!(
            "sleep 2; open -a {}",
            shell_quote(&bundle.to_string_lossy())
        );
        spawn_detached_shell(&script)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let exe = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .ok_or("could not find the app to restart")?;
        let script = format!("sleep 2; exec {}", shell_quote(&exe.to_string_lossy()));
        spawn_detached_shell(&script)
    }
    #[cfg(windows)]
    {
        // Nothing to do. The silent installer starts the tray itself once it
        // has replaced the files, which is the only moment at which the new
        // executable actually exists.
        Ok(())
    }
}

#[cfg(unix)]
fn spawn_detached_shell(script: &str) -> Result<(), String> {
    proc::quiet("/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not arrange the restart: {e}"))
}

/// Wrap a path so a shell treats it as one word whatever it contains.
///
/// The default macOS install path contains a space, so this is the normal case.
#[cfg(unix)]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(all(test, unix))]
mod shell_tests {
    use super::shell_quote;

    #[test]
    fn quotes_the_paths_we_actually_ship_to() {
        assert_eq!(
            shell_quote("/Applications/GCloud Dot.app"),
            "'/Applications/GCloud Dot.app'"
        );
    }

    #[test]
    fn a_quote_in_the_path_cannot_end_the_quoting() {
        // A home directory can contain an apostrophe, and that must not turn
        // the rest of the path into shell words.
        assert_eq!(
            shell_quote("/Users/o'brien/app"),
            r"'/Users/o'\''brien/app'"
        );
    }
}
