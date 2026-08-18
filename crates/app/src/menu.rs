//! Builds the tray menu from a [`Status`].
//!
//! This is what a click on the icon opens, so it stays short. It answers the
//! question you clicked to ask, offers the two things you might do about it,
//! and sends you to the window for anything longer.
//!
//! Rebuilt from scratch each time rather than mutated in place. The menu is
//! only read at the moment it opens, so rebuilding costs nothing you can
//! perceive, and it removes a whole class of bug where one branch forgets to
//! update a label.

use gcloud_dot_core::{
    credentials::AdcKind,
    settings::Theme,
    status::{AuthState, Status},
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
    pub const CHECK_FOR_UPDATES: &str = "set.updates";
    /// Prefix for configuration switching: `config:<name>`.
    pub const CONFIG_PREFIX: &str = "config:";
    /// Prefix for appearance: `theme:system`, `theme:light`, `theme:dark`.
    pub const THEME_PREFIX: &str = "theme:";
}

/// Stable identifiers for the appearance choices, so the menu id and the click
/// handler cannot drift apart.
pub fn theme_slug(t: Theme) -> &'static str {
    match t {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

pub fn theme_from_slug(slug: &str) -> Option<Theme> {
    Theme::ALL.iter().copied().find(|t| theme_slug(*t) == slug)
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

    // Line one answers the question you clicked to ask.
    match &status.auth {
        AuthState::Valid => info!(format!("{} Signed in as {account}", level.emoji())),
        AuthState::Expired => info!(format!("{} Signed out, {account}", level.emoji())),
        AuthState::Unknown(why) => info!(format!("{} {why}", level.emoji())),
    }

    if status.auth == AuthState::Valid {
        match status.remaining(now) {
            Some(left) if left.num_seconds() >= 0 => info!(format!(
                "About {} left ({})",
                gcloud_dot_core::status::long_duration(left),
                status.estimate.source.label()
            )),
            Some(_) => info!("Past the estimate, still valid"),
            None => info!("No login history yet"),
        }
    }

    if let Some(cfg) = &status.config {
        if let Some(project) = &cfg.project {
            info!(format!("Project: {project}"));
        }
    }

    // Application default credentials appear only when they are a problem.
    // Repeating "valid" every time you open the menu buys nothing, and this is
    // the credential behind code that fails while gcloud itself works.
    if settings.track_adc {
        if let (Some(file), Some(state)) = (&status.adc_file, &status.adc) {
            let word = match state {
                AuthState::Valid => None,
                AuthState::Expired => Some("expired"),
                AuthState::Unknown(_) => Some("unknown"),
            };
            if let Some(word) = word {
                let kind = match &file.kind {
                    AdcKind::UserCredentials => "user".to_string(),
                    AdcKind::ServiceAccount { client_email } => client_email
                        .split('@')
                        .next()
                        .unwrap_or("service account")
                        .to_string(),
                    AdcKind::Other { kind } => kind.clone(),
                };
                info!(format!("Application default credentials: {word} ({kind})"));
            }
        }
    }

    if let Some(file) = &status.adc_file {
        if gcloud_dot_core::credentials::key_file_is_stale(file, now, settings.sa_key_warn_days) {
            let days = gcloud_dot_core::credentials::age_days(file, now).unwrap_or_default();
            info!(format!("Service account key file is {days} days old"));
        }
    }

    if let Some(note) = &status.probe_note {
        info!(format!("Last check was inconclusive: {note}"));
    }
    if let Some(checked) = status.checked_at {
        info!(format!("Last checked at {}", checked.format("%H:%M:%S")));
    }

    // Everything past here is something you can do.
    owned.push(Box::new(PredefinedMenuItem::separator()));
    owned.push(Box::new(MenuItem::with_id(
        id::DETAILS,
        "Open window",
        true,
        None,
    )));
    owned.push(Box::new(MenuItem::with_id(
        id::LOGIN,
        "Sign in again",
        status.gcloud_found,
        None,
    )));
    owned.push(Box::new(MenuItem::with_id(
        id::CHECK,
        "Check now",
        status.gcloud_found,
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
        // `items` is deliberately dropped here. muda items are `Rc` backed and
        // the submenu keeps a clone, so nothing is lost. Pushing them into
        // `owned` as well would append every configuration a second time at the
        // top level, because everything in `owned` is appended below.
    }

    let launch = CheckMenuItem::with_id(
        id::LAUNCH_AT_LOGIN,
        "Launch at login",
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
        "Track application default credentials",
        true,
        settings.track_adc,
        None,
    );
    // The one setting here that reaches the network. Somebody who does not
    // want an app calling GitHub once a day should be able to say so without
    // editing a file they have to be told exists.
    let updates = CheckMenuItem::with_id(
        id::CHECK_FOR_UPDATES,
        "Check for updates",
        true,
        settings.check_for_updates,
        None,
    );

    // Appearance, on every platform. The window is often the only one open on a
    // machine whose global theme was set for something else.
    let theme_items: Vec<CheckMenuItem> = Theme::ALL
        .iter()
        .map(|t| {
            CheckMenuItem::with_id(
                format!("{}{}", id::THEME_PREFIX, theme_slug(*t)),
                t.label(),
                true,
                settings.theme == *t,
                None,
            )
        })
        .collect();
    let theme_refs: Vec<&dyn IsMenuItem> =
        theme_items.iter().map(|i| i as &dyn IsMenuItem).collect();
    let appearance = Submenu::with_items("Appearance", true, &theme_refs).ok();

    let mut settings_refs: Vec<&dyn IsMenuItem> =
        vec![&launch, &notifications, &track_adc, &updates];
    if let Some(sub) = &appearance {
        settings_refs.push(sub);
    }

    // macOS is the only platform with a text slot beside the icon, so this
    // toggle would mean nothing anywhere else.
    let countdown = CheckMenuItem::with_id(
        id::COUNTDOWN_TEXT,
        "Show countdown in menu bar",
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
            // Worded as the action it performs, not as an announcement. It
            // installs the release; it does not open a page about it.
            format!("Update to {version}"),
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
    fn theme_ids_round_trip() {
        for t in Theme::ALL {
            assert_eq!(theme_from_slug(theme_slug(t)), Some(t));
        }
        assert_eq!(theme_from_slug("chartreuse"), None);
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
            id::CHECK_FOR_UPDATES,
        ];
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }
}
