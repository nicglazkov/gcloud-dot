//! Retiring the apps this one replaces.
//!
//! GCloud Dot 1.0 keeps the bundle identifier and LaunchAgent label of the
//! shell-script-installed Swift app that preceded it, so that settings and
//! measured session lengths carry over. That sharing has one consequence worth
//! handling: until the old agent is stopped, both run, and the user sees two
//! dots reporting the same thing.
//!
//! Nothing is deleted. The old application bundle stays exactly where it is;
//! only its login item and its running process are retired, both of which this
//! app is about to provide itself.

/// The path the old installer always used.
#[cfg(target_os = "macos")]
const LEGACY_BINARY_SUFFIX: &str = "GCloud Dot.app/Contents/MacOS/GCloudDot";

/// Stop a previous-generation instance if one is running.
///
/// Returns a line describing what happened, for the log.
#[cfg(target_os = "macos")]
pub fn retire() -> Option<String> {
    let plist = dirs::home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", crate::autostart::LABEL));
    let body = std::fs::read_to_string(&plist).ok()?;
    if !body.contains(LEGACY_BINARY_SUFFIX) {
        return None; // Already ours, or absent.
    }

    // Unload first: without this launchd relaunches whatever we kill.
    let uid = unsafe { libc_getuid() };
    let _ = gcloud_dot_core::proc::quiet("launchctl")
        .args(["bootout", &format!("gui/{uid}/{}", crate::autostart::LABEL)])
        .output();
    let _ = gcloud_dot_core::proc::quiet("pkill")
        .args(["-f", LEGACY_BINARY_SUFFIX])
        .output();

    Some(
        "retired the previous GCloud Dot login item; its app bundle is still in \
         ~/Applications and can be deleted whenever you like"
            .to_string(),
    )
}

/// `getuid` without pulling in the `libc` crate for one call.
#[cfg(target_os = "macos")]
unsafe fn libc_getuid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    getuid()
}

/// The Windows tray this replaces installs a scheduled task rather than a login
/// item, so the same reasoning applies there.
#[cfg(target_os = "windows")]
pub fn retire() -> Option<String> {
    let query = gcloud_dot_core::proc::quiet("schtasks")
        .args(["/Query", "/TN", "GcloudAuthTray"])
        .output()
        .ok()?;
    if !query.status.success() {
        return None;
    }
    let _ = gcloud_dot_core::proc::quiet("schtasks")
        .args(["/End", "/TN", "GcloudAuthTray"])
        .output();
    let _ = gcloud_dot_core::proc::quiet("schtasks")
        .args(["/Delete", "/TN", "GcloudAuthTray", "/F"])
        .output();
    // The old tray is a hidden PowerShell host, identified by its script name.
    let _ = gcloud_dot_core::proc::quiet("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_Process -Filter \"Name='powershell.exe'\" | \
             Where-Object { $_.CommandLine -match 'GcloudAuthTray\\.ps1' } | \
             ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }",
        ])
        .output();
    Some("retired the previous GcloudAuthTray scheduled task".to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn retire() -> Option<String> {
    None // No predecessor shipped for Linux.
}
