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
