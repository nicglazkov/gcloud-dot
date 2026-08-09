//! Desktop notifications.
//!
//! Best effort by design. A notification that cannot be shown, no D-Bus on a
//! bare window manager, notifications denied on macOS, must never take the
//! tray down with it, because the icon is still doing its job.

use gcloud_dot_core::Urgency;

pub fn show(title: &str, body: &str, urgency: Urgency) {
    if let Err(e) = try_show(title, body, urgency) {
        eprintln!("gcloud-dot: could not post a notification: {e}");
    }
}

fn try_show(title: &str, body: &str, urgency: Urgency) -> Result<(), Box<dyn std::error::Error>> {
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(body);

    #[cfg(target_os = "windows")]
    {
        // Without an application identity, notify-rust falls back to
        // PowerShell's, and every toast is labelled "PowerShell". For an app
        // that watches credentials, a security prompt apparently from a shell
        // is worse than no notification at all.
        n.app_id(gcloud_dot_core::proc::WINDOWS_APP_ID);
    }

    #[cfg(target_os = "macos")]
    {
        // Notifications on macOS are attributed to a bundle. Ours is set here
        // so the alert says "GCloud Dot" rather than naming whatever process
        // happens to be hosting the delivery mechanism.
        let _ = notify_rust::set_application(crate::autostart::LABEL);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        n.appname("GCloud Dot");
        n.icon("gcloud-dot");
        n.urgency(match urgency {
            Urgency::Info => notify_rust::Urgency::Normal,
            // Critical notifications on Linux stay on screen until dismissed.
            // An expired session is exactly the case that warrants it: the
            // whole point is that you find out before your next command fails.
            Urgency::Warning => notify_rust::Urgency::Critical,
        });
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let _ = urgency;

    n.show()?;
    Ok(())
}

/// Teach Windows the name and icon to show on this app's notifications.
///
/// An unpackaged desktop application has no manifest for Windows to read, so
/// the display name and icon are looked up in the registry under the
/// application identity. Writing them is all that stands between a toast that
/// says "GCloud Dot" and one that says "PowerShell".
///
/// Written on every start because it costs nothing and repairs itself after a
/// profile is reset or a half finished uninstall.
#[cfg(target_os = "windows")]
pub fn register_windows_identity() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    let path = format!(
        "Software\\Classes\\AppUserModelId\\{}",
        gcloud_dot_core::proc::WINDOWS_APP_ID
    );
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok((key, _)) = hkcu.create_subkey_with_flags(&path, KEY_WRITE) else {
        return;
    };
    let _ = key.set_value("DisplayName", &"GCloud Dot");
    if let Ok(exe) = std::env::current_exe() {
        // The icon ships beside the executable, put there by the installer.
        let icon = exe.with_file_name("gcloud-dot.ico");
        if icon.exists() {
            let _ = key.set_value("IconUri", &icon.to_string_lossy().to_string());
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn register_windows_identity() {}
