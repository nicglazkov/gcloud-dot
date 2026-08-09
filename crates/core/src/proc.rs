//! Spawning child processes without a console window appearing.
//!
//! On Windows a console subprocess started by a windowed parent allocates its
//! own console, so every child flashes a black window and steals focus. A tray
//! app that probes gcloud every ten minutes, rescans logs every minute, and
//! checks for updates once a day would blink at you all day.
//!
//! `CREATE_NO_WINDOW` is the flag that suppresses it, and it is the right one
//! to reach for. The alternatives that look equivalent are not: launching
//! through `wscript` or `powershell -WindowStyle Hidden -EncodedCommand` is the
//! shape of a malware dropper, and Defender scores it accordingly. Running a
//! normal, readable command line with the console suppressed is not.
//!
//! The flag is inert on every other platform, so there is no reason to spawn
//! any other way from a windowed process.

use std::process::Command;

/// A `Command` that never allocates a console on Windows.
///
/// Use this for every child process started by the tray. The command line
/// stays plain and readable, which is what keeps it from looking like
/// something worth quarantining.
pub fn quiet<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(program);
    hide_console(&mut cmd);
    cmd
}

/// Suppress the console for a `Command` that already exists.
pub fn hide_console(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// The identity Windows uses to attribute a notification to an application.
///
/// Without one, `notify-rust` falls back to PowerShell's identity and every
/// toast is labelled "PowerShell", which is both wrong and alarming for an app
/// that watches credentials.
///
/// Registering it needs no installer and no packaging: Windows reads the
/// display name and icon for an unpackaged application straight out of
/// `HKCU\Software\Classes\AppUserModelId\<id>`.
pub const WINDOWS_APP_ID: &str = "nicglazkov.GCloudDot";
