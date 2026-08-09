//! Things the user asks for from the menu or the panel.
//!
//! Signing in is deliberately handed to a visible terminal rather than run
//! silently in the background. `gcloud auth login` asks questions, prints a URL
//! that sometimes has to be copied by hand, and hands off to a browser; running
//! that where nobody can see it produces a hang with no explanation.

use std::path::Path;
use std::process::{Command, Stdio};

/// Open `gcloud auth login` in a terminal the user can watch.
pub fn login(gcloud: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // A .command file opened with `open` needs no automation permission.
        // Driving Terminal with AppleScript would trigger a TCC prompt, and
        // this app is otherwise permission-free.
        let script = format!(
            "#!/bin/zsh\nclear\necho \"Signing in to gcloud\"\necho\n{} auth login\necho\n\
             echo \"Done. The dot updates within a few seconds.\"\necho \"You can close this window.\"\n",
            shell_quote(&gcloud.to_string_lossy())
        );
        let path = std::env::temp_dir().join("gcloud-dot-signin.command");
        std::fs::write(&path, script)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        Command::new("open").arg(&path).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // /k keeps the window open so any error stays readable.
        Command::new("cmd.exe")
            .args(["/c", "start", "", "cmd.exe", "/k"])
            .arg(format!("\"{}\" auth login", gcloud.display()))
            .spawn()?;
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let command = format!(
            "{} auth login; echo; read -p 'Press enter to close'",
            shell_quote(&gcloud.to_string_lossy())
        );
        // No portable way to ask for "the user's terminal", so try the ones
        // that actually exist in the wild, most standard first.
        let terminals: [(&str, Vec<&str>); 6] = [
            ("x-terminal-emulator", vec!["-e", "sh", "-c"]),
            ("gnome-terminal", vec!["--", "sh", "-c"]),
            ("konsole", vec!["-e", "sh", "-c"]),
            ("xfce4-terminal", vec!["-x", "sh", "-c"]),
            ("alacritty", vec!["-e", "sh", "-c"]),
            ("xterm", vec!["-e", "sh", "-c"]),
        ];
        for (bin, args) in terminals {
            let mut cmd = Command::new(bin);
            cmd.args(&args).arg(&command);
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        // Headless or an unusual desktop: run it directly. gcloud falls back to
        // printing a URL, which is still usable if stdout goes to a journal.
        Command::new(gcloud)
            .args(["auth", "login"])
            .stdin(Stdio::null())
            .spawn()?;
        Ok(())
    }
}

/// Switch the active gcloud configuration.
pub fn activate_config(gcloud: &Path, name: &str) -> std::io::Result<()> {
    let status = Command::new(gcloud)
        .args(["config", "configurations", "activate", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "gcloud refused to activate {name}"
        )))
    }
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd.exe");
        c.args(["/c", "start", ""]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = Command::new("xdg-open");

    cmd.arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(unix)]
fn shell_quote(s: &str) -> String {
    // Single quotes with an escape for embedded single quotes: the one form
    // that is safe for every path a user can actually have.
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn paths_with_awkward_characters_survive_quoting() {
        use super::shell_quote;
        assert_eq!(
            shell_quote("/usr/local/bin/gcloud"),
            "'/usr/local/bin/gcloud'"
        );
        assert_eq!(
            shell_quote("/Users/o'brien/sdk/gcloud"),
            r"'/Users/o'\''brien/sdk/gcloud'"
        );
        // A space is the common case on macOS and must not split the command.
        assert!(shell_quote("/Volumes/My Disk/gcloud").starts_with('\''));
    }
}
