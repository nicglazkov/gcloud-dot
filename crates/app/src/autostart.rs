//! Starting at login, the way each platform expects.
//!
//! Three genuinely different mechanisms, kept behind one two-function
//! interface. Each writes only inside the user's own account, so none of this
//! ever needs administrator rights.

use std::path::PathBuf;

/// The LaunchAgent label, which is also the bundle identifier, shared with the
/// app this one replaces so macOS treats the upgrade as the same application
/// rather than a second one.
///
/// Only macOS has anything to name: Windows uses a Startup shortcut and Linux a
/// desktop entry, neither of which carries an identifier.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const LABEL: &str = "com.nic.gclouddot";

/// Is the app registered to start at login?
pub fn is_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        plist_path().is_some_and(|p| p.exists())
    }
    #[cfg(target_os = "windows")]
    {
        startup_shortcut().is_some_and(|p| p.exists())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        desktop_entry_path().is_some_and(|p| p.exists())
    }
}

pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    if enabled {
        enable()
    } else {
        disable()
    }
}

fn executable() -> std::io::Result<PathBuf> {
    std::env::current_exe()
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"))
    })
}

#[cfg(target_os = "macos")]
fn enable() -> std::io::Result<()> {
    let exe = executable()?;
    let path = plist_path().ok_or_else(|| std::io::Error::other("no home directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // RunAtLoad only. No KeepAlive: if the user quits from the menu, the app
    // should stay quit until the next login rather than immediately reappear.
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#,
        exe.display()
    );
    std::fs::write(&path, plist)
}

#[cfg(target_os = "macos")]
fn disable() -> std::io::Result<()> {
    if let Some(path) = plist_path() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

// -------------------------------------------------------------- Windows

/// A shortcut in the per-user Startup folder.
///
/// Chosen over a `Run` registry value because it is visible in Task Manager's
/// Startup tab, where a user who wants this gone will look for it first.
#[cfg(target_os = "windows")]
fn startup_shortcut() -> Option<PathBuf> {
    dirs::config_dir().map(|d| {
        d.join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join("GCloud Dot.lnk")
    })
}

#[cfg(target_os = "windows")]
fn enable() -> std::io::Result<()> {
    let exe = executable()?;
    let link = startup_shortcut().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Writing a .lnk by hand means implementing the Shell Link binary format;
    // asking the shell to do it is three lines and always correct.
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.Description = 'GCloud Dot'; \
         $s.Save()",
        link.display(),
        exe.display()
    );
    // A plain, readable command line with the console suppressed. Not
    // `-WindowStyle Hidden` and not `-EncodedCommand`: those two are the shape
    // Defender scores as a dropper, and neither is needed to avoid the flash.
    let status = gcloud_dot_core::proc::quiet("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "could not create the startup shortcut",
        ))
    }
}

#[cfg(target_os = "windows")]
fn disable() -> std::io::Result<()> {
    if let Some(link) = startup_shortcut() {
        if link.exists() {
            std::fs::remove_file(link)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- Linux

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_entry_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart").join("gcloud-dot.desktop"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn enable() -> std::io::Result<()> {
    let exe = executable()?;
    let path = desktop_entry_path().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=GCloud Dot\n\
         Comment=Shows how long your gcloud auth session has left\n\
         Exec={}\n\
         Icon=gcloud-dot\n\
         Terminal=false\n\
         Categories=Utility;Development;\n\
         X-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    std::fs::write(&path, entry)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn disable() -> std::io::Result<()> {
    if let Some(path) = desktop_entry_path() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_do_not_panic_on_any_platform() {
        let _ = is_enabled();
    }

    #[test]
    fn the_label_matches_the_bundle_identifier() {
        // The LaunchAgent label and the bundle id must agree, or macOS will
        // treat a relaunch as a different app and leave two dots in the bar.
        assert_eq!(LABEL, "com.nic.gclouddot");
    }
}
