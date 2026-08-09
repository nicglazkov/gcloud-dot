//! Builds the tray menu from a [`Status`].
//!
//! Rebuilt from scratch each time it is opened rather than mutated in place.
//! The menu is only ever read at the moment of opening, so rebuilding costs
//! nothing a user can perceive and removes a whole category of bug where one
//! branch forgets to update a label.

use gcloud_dot_core::{
    credentials::AdcKind,
    status::{ago, AuthState, Status},
    Settings,
};
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

/// Menu item identifiers. Strings so the event handler reads as prose.
pub mod id {
    pub const LOGIN: &str = "login";
    pub const CHECK: &str = "check";
    pub const DETAILS: &str = "details";
    pub const QUIT: &str = "quit";
    pub const WEBSITE: &str = "website";
    pub const UPDATE: &str = "update";
    pub const LAUNCH_AT_LOGIN: &str = "set.launch";
    pub const NOTIFICATIONS: &str = "set.notifications";
    pub const COUNTDOWN_TEXT: &str = "set.countdown";
    pub const TRACK_ADC: &str = "set.adc";
    /// Prefix for configuration switching: `config:<name>`.
    pub const CONFIG_PREFIX: &str = "config:";
}

pub struct MenuModel {
    pub menu: Menu,
}

/// Build the whole menu.
pub fn build(
    status: &Status,
    settings: &Settings,
    configurations: &[String],
    update_available: Option<&str>,
) -> MenuModel {
    let now = chrono::Local::now();
    let menu = Menu::new();
    let mut owned: Vec<Box<dyn IsMenuItem>> = Vec::new();

    // A disabled item is the only way muda offers to show a line of text, so
    // the informational rows are non-interactive menu items.
    macro_rules! info {
        ($text:expr) => {
            owned.push(Box::new(MenuItem::new($text, false, None)))
        };
    }

    let level = status.level(now);
    let account = status
        .config
        .as_ref()
        .and_then(|c| c.account.clone())
        .unwrap_or_else(|| "no account".into());

    match &status.auth {
        AuthState::Valid => info!(format!("{} Signed in as {account}", level.emoji())),
        AuthState::Expired => info!(format!("{} Signed out — {account}", level.emoji())),
        AuthState::Unknown(why) => info!(format!("{} {why}", level.emoji())),
    }

    if status.auth == AuthState::Valid {
        match status.remaining(now) {
            Some(left) if left.num_seconds() >= 0 => info!(format!(
                "About {} left ({})",
                gcloud_dot_core::status::long_duration(left),
                status.estimate.source.label()
            )),
            Some(_) => info!("Past the estimate — still valid"),
            None => info!("No login history yet"),
        }
    }
    if let Some(note) = &status.probe_note {
        info!(format!("Last check inconclusive: {note}"));
    }

    owned.push(Box::new(PredefinedMenuItem::separator()));

    if let Some(cfg) = &status.config {
        if let Some(project) = &cfg.project {
            info!(format!("Project: {project}"));
        }
    }
    if let Some(start) = status.session_start {
        info!(format!(
            "Last login: {} ({})",
            start.format("%a %H:%M"),
            ago(start, now)
        ));
    }
    if let Some(expiry) = status.predicted_expiry() {
        if status.auth == AuthState::Valid {
            info!(format!("Est. re-auth: {}", expiry.format("%a %H:%M")));
        }
    }

    // ADC gets its own line because it is a different credential that fails
    // separately, which is the whole reason it is tracked.
    if settings.track_adc {
        if let (Some(file), Some(state)) = (&status.adc_file, &status.adc) {
            let kind = match &file.kind {
                AdcKind::UserCredentials => "user".to_string(),
                AdcKind::ServiceAccount { client_email } => client_email
                    .split('@')
                    .next()
                    .unwrap_or("service account")
                    .to_string(),
                AdcKind::Other { kind } => kind.clone(),
            };
            let word = match state {
                AuthState::Valid => "valid",
                AuthState::Expired => "expired",
                AuthState::Unknown(_) => "unknown",
            };
            info!(format!("ADC: {word} ({kind})"));
        }
    }

    if let Some(file) = &status.adc_file {
        if gcloud_dot_core::credentials::key_file_is_stale(file, now, settings.sa_key_warn_days) {
            let days = gcloud_dot_core::credentials::age_days(file, now).unwrap_or_default();
            info!(format!("⚠ Service account key file is {days} days old"));
        }
    }

    if let Some(checked) = status.checked_at {
        info!(format!("Last checked: {}", checked.format("%H:%M:%S")));
    }

    owned.push(Box::new(PredefinedMenuItem::separator()));
    owned.push(Box::new(MenuItem::with_id(
        id::LOGIN,
        "Sign In Again…",
        status.gcloud_found,
        None,
    )));
    owned.push(Box::new(MenuItem::with_id(
        id::CHECK,
        "Check Now",
        status.gcloud_found,
        None,
    )));
    owned.push(Box::new(MenuItem::with_id(
        id::DETAILS,
        "Details…",
        true,
        None,
    )));

    // Configuration switcher, only when there is more than one to switch to.
    if configurations.len() > 1 {
        let active = status.config.as_ref().map(|c| c.name.as_str());
        let items: Vec<CheckMenuItem> = configurations
            .iter()
            .map(|name| {
                CheckMenuItem::with_id(
                    format!("{}{name}", id::CONFIG_PREFIX),
                    name,
                    true,
                    Some(name.as_str()) == active,
                    None,
                )
            })
            .collect();
        let refs: Vec<&dyn IsMenuItem> = items.iter().map(|i| i as &dyn IsMenuItem).collect();
        if let Ok(sub) = Submenu::with_items("Configuration", true, &refs) {
            owned.push(Box::new(sub));
        }
        // `items` is deliberately dropped here. muda items are `Rc`-backed and
        // the submenu keeps a clone, so nothing is lost — and pushing them into
        // `owned` as well would append every configuration a second time at the
        // top level of the menu, since everything in `owned` is appended below.
    }

    // Settings submenu.
    let launch = CheckMenuItem::with_id(
        id::LAUNCH_AT_LOGIN,
        "Launch at Login",
        true,
        settings.launch_at_login,
        None,
    );
    let notifications = CheckMenuItem::with_id(
        id::NOTIFICATIONS,
        "Notifications",
        true,
        settings.notifications_enabled,
        None,
    );
    let track_adc = CheckMenuItem::with_id(
        id::TRACK_ADC,
        "Track Application Default Credentials",
        true,
        settings.track_adc,
        None,
    );
    let mut settings_refs: Vec<&dyn IsMenuItem> = vec![&launch, &notifications, &track_adc];

    // macOS is the only platform with a text slot beside the icon, so this
    // toggle would mean nothing anywhere else.
    let countdown = CheckMenuItem::with_id(
        id::COUNTDOWN_TEXT,
        "Show Countdown in Menu Bar",
        true,
        settings.show_countdown_text,
        None,
    );
    if cfg!(target_os = "macos") {
        settings_refs.push(&countdown);
    }

    if let Ok(sub) = Submenu::with_items("Settings", true, &settings_refs) {
        owned.push(Box::new(sub));
    }

    owned.push(Box::new(PredefinedMenuItem::separator()));
    if let Some(version) = update_available {
        owned.push(Box::new(MenuItem::with_id(
            id::UPDATE,
            format!("Update available: {version}"),
            true,
            None,
        )));
    }
    owned.push(Box::new(MenuItem::with_id(
        id::WEBSITE,
        format!("GCloud Dot {}", gcloud_dot_core::VERSION),
        true,
        None,
    )));
    owned.push(Box::new(MenuItem::with_id(
        id::QUIT,
        "Quit GCloud Dot",
        true,
        None,
    )));

    for item in &owned {
        let _ = menu.append(item.as_ref());
    }

    MenuModel { menu }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_ids_round_trip() {
        let id = format!("{}{}", id::CONFIG_PREFIX, "work");
        assert_eq!(id.strip_prefix(id::CONFIG_PREFIX), Some("work"));
        // A configuration named like another id must not be mistaken for one.
        let odd = format!("{}{}", id::CONFIG_PREFIX, id::QUIT);
        assert_eq!(odd.strip_prefix(id::CONFIG_PREFIX), Some(id::QUIT));
    }

    #[test]
    fn ids_are_distinct() {
        let all = [
            id::LOGIN,
            id::CHECK,
            id::DETAILS,
            id::QUIT,
            id::WEBSITE,
            id::UPDATE,
            id::LAUNCH_AT_LOGIN,
            id::NOTIFICATIONS,
            id::COUNTDOWN_TEXT,
            id::TRACK_ADC,
        ];
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }
}
