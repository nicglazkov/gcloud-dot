//! GCloud Dot, the menu bar and system tray app.
//!
//! Structure: one event loop on the main thread owns all UI, and every
//! blocking operation (running gcloud, walking the log directory) happens on a
//! short-lived worker thread that reports back through the loop's proxy. The
//! decisions themselves live in `gcloud-dot-core`, which has no idea any of
//! this exists.

// No console window on Windows: this is a tray app, and a flashing black
// rectangle at every login is the most visible bug a user would ever see.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod icon;
mod legacy;
mod menu;
mod notify;
mod panel;
mod single_instance;
mod update;

use chrono::{DateTime, Local};
use gcloud_dot_core::{
    engine::{Engine, Event as CoreEvent, Plan},
    gcloud, logs, paths,
    probe::{self, Credential, ProbeOutcome},
    status::Level,
};
use std::path::PathBuf;
use std::time::Duration;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Window, WindowBuilder};
use tray_icon::{menu::MenuEvent, TrayIcon, TrayIconBuilder};
use wry::WebView;

/// How often the loop wakes to consider doing something. The engine decides
/// whether anything is actually due, so this only bounds latency.
const TICK: Duration = Duration::from_secs(5);

#[derive(Debug)]
enum UserEvent {
    Tick,
    Work(Box<WorkResult>),
    Menu(String),
    Panel(String),
    UpdateFound(String),
    /// One step of an upgrade in progress.
    UpdateProgress(String),
    /// An upgrade attempt has finished, one way or the other.
    UpdateDone(Box<Result<gcloud_dot_core::upgrade::Outcome, String>>),
    /// The replacement is on disk. Stand down so the new copy can take over.
    UpdateRestart,
    LoginFailed(String),
}

#[derive(Debug, Default)]
struct WorkResult {
    user: Option<ProbeOutcome>,
    adc: Option<ProbeOutcome>,
    logins: Option<Vec<DateTime<Local>>>,
}

/// What the tray currently shows. Redrawing is skipped unless this changes.
///
/// On Linux this is not just an optimisation: `tray-icon` sets an AppIndicator
/// icon by writing a file and pointing the indicator at it, so an unguarded
/// redraw every five seconds would churn twelve temp files a minute forever.
#[derive(PartialEq, Clone)]
struct RenderKey {
    level: Level,
    label: String,
    title: String,
    tooltip: String,
}

struct App {
    engine: Engine,
    gcloud_path: Option<PathBuf>,
    configurations: Vec<String>,
    tray: Option<TrayIcon>,
    panel: Option<(Window, WebView)>,
    last_render: Option<RenderKey>,
    work_in_flight: bool,
    update_available: Option<String>,
    update_checks_on: std::sync::Arc<std::sync::atomic::AtomicBool>,
    update_ui: update::UpdateUi,
    state_path: PathBuf,
    proxy: tao::event_loop::EventLoopProxy<UserEvent>,
}

fn main() {
    // Held for the process lifetime; the OS releases it however we exit.
    let Some(_guard) = single_instance::acquire() else {
        eprintln!("GCloud Dot is already running.");
        return;
    };

    let (state, migrated) = gcloud_dot_core::load_state();
    if let Some(note) = &migrated {
        eprintln!("gcloud-dot: {note}");
    }

    // Do this before anything can raise a notification.
    notify::register_windows_identity();
    // Anything a previous upgrade could not delete because it was still running.
    gcloud_dot_core::upgrade::clear_replaced_files();

    // Before touching the login item, stand down whatever came before. Both
    // predecessors share this app's identity, so leaving one running puts two
    // dots in the bar saying the same thing.
    if let Some(note) = legacy::retire() {
        eprintln!("gcloud-dot: {note}");
    }

    // Make the setting true of the machine, not just of the file. Without this
    // the default never takes effect on a fresh install, and a login item the
    // user removed by hand would silently disagree with the menu's checkmark.
    if state.settings.launch_at_login != autostart::is_enabled() {
        if let Err(e) = autostart::set_enabled(state.settings.launch_at_login) {
            eprintln!("gcloud-dot: could not reconcile launch at login: {e}");
        }
    }

    let mut engine = Engine::new(state);
    let gcloud_path = gcloud::find();
    let (config, adc_file) = gcloud_dot_core::read_environment();
    engine.set_environment(gcloud_path.is_some(), config, adc_file);
    engine.refresh_estimate();

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        // Accessory: a menu bar presence with no Dock icon, and no menu bar
        // takeover when the details window takes focus. `LSUIElement` in the
        // bundle says the same thing, but this also holds for a bare binary.
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();

    // muda and tray-icon publish to process-global channels; forward both into
    // the loop so there is exactly one place where events are handled.
    {
        let proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(e.id.0.clone()));
        }));
    }
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(TICK);
            if proxy.send_event(UserEvent::Tick).is_err() {
                return;
            }
        });
    }

    // Shared with the worker below, so the menu can turn it off without
    // needing to reach into a thread.
    let update_checks_on = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        engine.state.settings.check_for_updates,
    ));
    {
        let proxy = proxy.clone();
        // Delayed so a login-time launch does not race the network coming up.
        update::check_in_background(
            Duration::from_secs(20),
            update_checks_on.clone(),
            move |v| {
                let _ = proxy.send_event(UserEvent::UpdateFound(v));
            },
        );
    }

    let mut app = App {
        engine,
        gcloud_path,
        configurations: Vec::new(),
        tray: None,
        panel: None,
        last_render: None,
        work_in_flight: false,
        update_available: None,
        update_checks_on: update_checks_on.clone(),
        update_ui: update::UpdateUi::default(),
        state_path: paths::state_path(),
        proxy: proxy.clone(),
    };
    app.reload_configurations();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                // macOS needs NSApplication to exist before a status item can
                // be created, which is only guaranteed once the loop starts.
                app.build_tray();
                app.refresh_ui();
                app.dispatch_work(
                    &proxy,
                    Plan {
                        probe_user: true,
                        probe_adc: app.engine.state.settings.track_adc,
                        rescan_logs: true,
                    },
                );
            }

            Event::UserEvent(UserEvent::Tick) => {
                // Checked before anything else: when the menu bar is full the
                // icon is not drawn and its Quit item cannot be reached, so
                // `gcloud-dot quit` is the only way out.
                if gcloud_dot_core::take_quit_request() {
                    app.save();
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                let plan = app.engine.plan(Local::now());
                app.dispatch_work(&proxy, plan);
                // Redraw regardless: the countdown advances on its own.
                app.refresh_ui();
            }

            Event::UserEvent(UserEvent::Work(result)) => {
                app.work_in_flight = false;
                app.apply_work(*result);
                app.refresh_ui();
                // The menu shows the last-checked time and the ADC state, and
                // neither appears in the tooltip that `refresh_ui` compares
                // against, so a probe that changed only those would otherwise
                // leave the menu quoting the previous check.
                app.rebuild_menu();
                app.refresh_panel();
            }

            Event::UserEvent(UserEvent::Menu(id)) => {
                app.on_menu(&id, target, control_flow, &proxy);
            }

            Event::UserEvent(UserEvent::Panel(message)) => {
                app.on_panel_message(&message, target, &proxy);
            }

            // gcloud runs with no window, so a failure has nowhere to appear
            // unless the app says so itself.
            Event::UserEvent(UserEvent::LoginFailed(reason)) => {
                notify::show(
                    "Sign in did not finish",
                    &format!(
                        "{reason}\nFull output: {}",
                        actions::login_log_path().display()
                    ),
                    gcloud_dot_core::Urgency::Warning,
                );
            }

            Event::UserEvent(UserEvent::UpdateFound(version)) => {
                // The check repeats daily, so this arrives again every day that
                // an update goes uninstalled. Say it once per version: a
                // notification a day about the same release is how a user
                // learns to dismiss this app's notifications without reading
                // them, and the ones about expiring credentials are the point.
                let already_said = app.update_available.as_deref() == Some(version.as_str());
                if !already_said {
                    notify::show(
                        &format!("GCloud Dot {version} is available"),
                        gcloud_dot_core::upgrade::notification_body(
                            gcloud_dot_core::upgrade::detect(),
                        ),
                        gcloud_dot_core::Urgency::Info,
                    );
                }
                app.update_available = Some(version.clone());
                // The banner is set regardless, because it is not an
                // interruption. It is only a statement of what is true, and it
                // has to survive being dismissed by an upgrade that failed.
                if !app.update_ui.is_busy() {
                    app.update_ui = update::UpdateUi::Available(version);
                }
                app.rebuild_menu();
                app.refresh_panel();
            }

            Event::UserEvent(UserEvent::UpdateProgress(step)) => {
                app.update_ui = update::UpdateUi::Working(step);
                app.refresh_panel();
            }

            Event::UserEvent(UserEvent::UpdateDone(result)) => {
                app.update_ui = update::UpdateUi::from_outcome(*result);
                if matches!(app.update_ui, update::UpdateUi::Restarting(_)) {
                    app.update_available = None;
                    // Long enough for the banner to be read, short enough that
                    // nobody wonders whether it worked.
                    let proxy = proxy.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(2));
                        let _ = proxy.send_event(UserEvent::UpdateRestart);
                    });
                }
                app.rebuild_menu();
                app.refresh_panel();
            }

            Event::UserEvent(UserEvent::UpdateRestart) => {
                if let Err(e) = gcloud_dot_core::upgrade::schedule_relaunch() {
                    // Nothing will bring it back, so do not disappear silently.
                    eprintln!("gcloud-dot: could not arrange the restart: {e}");
                    app.update_ui = update::UpdateUi::Failed(format!(
                        "The new version is installed, but GCloud Dot could not restart itself: {e}"
                    ));
                    app.refresh_panel();
                    return;
                }
                app.save();
                *control_flow = ControlFlow::Exit;
            }

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                app.close_panel(target);
            }

            _ => {}
        }
    });
}

impl App {
    fn build_tray(&mut self) {
        let bitmap = icon::render(None, Level::Unknown, icon::native_size());
        let image = tray_icon::Icon::from_rgba(bitmap.rgba, bitmap.width, bitmap.height).ok();
        let model = menu::build(
            &self.engine.status,
            &self.engine.state.settings,
            &self.configurations,
            None,
        );
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(model.menu))
            .with_tooltip("GCloud Dot");
        if let Some(image) = image {
            builder = builder.with_icon(image);
        }
        match builder.build() {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => {
                eprintln!("gcloud-dot: could not create a tray icon: {e}");
                eprintln!("{}", linux_tray_hint());
            }
        }
    }

    fn reload_configurations(&mut self) {
        self.configurations = paths::gcloud_config_dir()
            .map(|d| gcloud_dot_core::config::list(&d))
            .unwrap_or_default();
    }

    /// Start the blocking half of a tick on a worker thread.
    fn dispatch_work(&mut self, proxy: &tao::event_loop::EventLoopProxy<UserEvent>, plan: Plan) {
        if self.work_in_flight || (!plan.probe_user && !plan.probe_adc && !plan.rescan_logs) {
            return;
        }
        self.work_in_flight = true;
        let gcloud_path = self.gcloud_path.clone();
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            let mut result = WorkResult::default();
            if plan.rescan_logs {
                // Always answer, even with nothing, so the engine can record
                // that a scan happened. Returning `None` when gcloud has no log
                // directory would leave the scan permanently overdue and spawn
                // one of these threads every five seconds forever.
                result.logins = Some(match paths::gcloud_log_dir() {
                    Some(dir) => logs::scan_logins(&dir, None),
                    None => Vec::new(),
                });
            }
            if let Some(path) = &gcloud_path {
                let timeout = Duration::from_secs(25);
                if plan.probe_user {
                    result.user = Some(probe::run(path, Credential::User, timeout));
                }
                if plan.probe_adc {
                    result.adc = Some(probe::run(path, Credential::ApplicationDefault, timeout));
                }
            }
            let _ = proxy.send_event(UserEvent::Work(Box::new(result)));
        });
    }

    fn apply_work(&mut self, result: WorkResult) {
        let now = Local::now();
        let mut events = Vec::new();

        // Logins first: a login discovered in the same pass as a probe should
        // reset the session before the probe result is interpreted against it.
        if let Some(logins) = result.logins {
            events.extend(self.engine.apply_logins(&logins, now));
        }
        if let Some(outcome) = result.user {
            events.extend(self.engine.apply_user_probe(outcome, now));
        }
        if let Some(outcome) = result.adc {
            self.engine.apply_adc_probe(outcome);
        }

        // Environment is cheap to re-read and can change without a probe, e.g.
        // someone runs `gcloud config set project` in a terminal.
        let (config, adc_file) = gcloud_dot_core::read_environment();
        let gcloud_found = self.gcloud_path.is_some();
        self.engine.set_environment(gcloud_found, config, adc_file);

        let mut persist = false;
        for event in events {
            match event {
                CoreEvent::Notify {
                    title,
                    body,
                    urgency,
                } => notify::show(&title, &body, urgency),
                CoreEvent::Persist => persist = true,
            }
        }
        if persist {
            self.save();
        }
    }

    fn save(&self) {
        if let Err(e) = self.engine.state.save(&self.state_path) {
            eprintln!("gcloud-dot: could not save state: {e}");
        }
    }

    fn refresh_ui(&mut self) {
        let now = Local::now();
        let status = &self.engine.status;
        let level = status.level(now);
        let label = status.icon_label(now);

        // macOS has a text slot beside the icon; the other two do not, so the
        // same string goes into the bitmap instead.
        let title = menu_bar_title(status, &self.engine.state.settings, now);
        let key = RenderKey {
            level,
            label: label.clone(),
            title: title.clone(),
            tooltip: status.summary(now),
        };
        if self.last_render.as_ref() == Some(&key) {
            return;
        }

        if let Some(tray) = &self.tray {
            let text = icon::text_goes_in_icon().then_some(label.as_str());
            let bitmap = icon::render(text, level, icon::native_size());
            if let Ok(image) = tray_icon::Icon::from_rgba(bitmap.rgba, bitmap.width, bitmap.height)
            {
                let _ = tray.set_icon(Some(image));
            }
            // Always Some, never None. tray-icon's macOS set_title ignores a
            // None entirely rather than clearing, so passing it leaves the last
            // title on the button forever. That is how a red dot ended up next
            // to a stale "0m": the countdown was written while the session was
            // still valid, and nothing ever took it off again.
            tray.set_title(Some(&title));
            let _ = tray.set_tooltip(Some(&key.tooltip));
        }

        self.last_render = Some(key);

        // Rebuilt whenever anything drawn has changed, not only on a colour
        // change. muda has no will-open hook, so a menu built once would still
        // be claiming "about 14h left" three hours later, and the menu is the
        // only place the account, project, and estimate are readable at all.
        //
        // The tooltip carries minute precision, so in practice this fires about
        // once a minute while a countdown is running, and not at all otherwise.
        self.rebuild_menu();
    }

    fn rebuild_menu(&mut self) {
        let model = menu::build(
            &self.engine.status,
            &self.engine.state.settings,
            &self.configurations,
            self.update_available.as_deref(),
        );
        if let Some(tray) = &self.tray {
            tray.set_menu(Some(Box::new(model.menu)));
        }
    }

    fn on_menu(
        &mut self,
        id: &str,
        target: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
        control_flow: &mut ControlFlow,
        proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    ) {
        match id {
            menu::id::QUIT => {
                self.save();
                *control_flow = ControlFlow::Exit;
            }
            menu::id::LOGIN => self.start_login(proxy),
            menu::id::CHECK => {
                self.engine.begin_fast_poll(Local::now());
                let plan = Plan {
                    probe_user: true,
                    probe_adc: self.engine.state.settings.track_adc,
                    rescan_logs: true,
                };
                self.dispatch_work(proxy, plan);
            }
            menu::id::DETAILS => self.toggle_panel(target),
            menu::id::WEBSITE => {
                let _ = actions::open_url("https://nicglazkov.github.io/gcloud-dot/");
            }
            // Opening the window as well as starting the work: the upgrade
            // has steps and can fail, and a menu item that closes itself and
            // then does something for thirty seconds in silence is how a user
            // ends up clicking it four times.
            menu::id::UPDATE => {
                if self.panel.is_none() {
                    self.open_panel(target);
                }
                self.start_upgrade(proxy);
            }
            menu::id::LAUNCH_AT_LOGIN => {
                let now_on = !self.engine.state.settings.launch_at_login;
                match autostart::set_enabled(now_on) {
                    Ok(()) => {
                        self.engine.state.settings.launch_at_login = now_on;
                        self.save();
                    }
                    Err(e) => eprintln!("gcloud-dot: could not change launch at login: {e}"),
                }
                self.rebuild_menu();
            }
            menu::id::NOTIFICATIONS => {
                self.engine.state.settings.notifications_enabled ^= true;
                self.save();
                self.rebuild_menu();
            }
            menu::id::COUNTDOWN_TEXT => {
                self.engine.state.settings.show_countdown_text ^= true;
                self.save();
                self.last_render = None; // force a redraw
                self.refresh_ui();
                self.rebuild_menu();
            }
            menu::id::CHECK_FOR_UPDATES => {
                let now_on = !self.engine.state.settings.check_for_updates;
                self.engine.state.settings.check_for_updates = now_on;
                // The worker reads this rather than being started and stopped,
                // so switching off takes effect on its next wake.
                self.update_checks_on
                    .store(now_on, std::sync::atomic::Ordering::Relaxed);
                if !now_on {
                    // Stop saying a newer version exists once asked not to look.
                    self.update_available = None;
                    if !self.update_ui.is_busy() {
                        self.update_ui = update::UpdateUi::Nothing;
                    }
                    self.refresh_panel();
                }
                self.save();
                self.rebuild_menu();
            }
            menu::id::TRACK_ADC => {
                self.engine.state.settings.track_adc ^= true;
                if !self.engine.state.settings.track_adc {
                    self.engine.status.adc = None;
                }
                self.save();
                self.rebuild_menu();
            }
            other => {
                if let Some(name) = other.strip_prefix(menu::id::CONFIG_PREFIX) {
                    self.switch_configuration(name, proxy);
                } else if let Some(slug) = other.strip_prefix(menu::id::THEME_PREFIX) {
                    if let Some(theme) = menu::theme_from_slug(slug) {
                        self.engine.state.settings.theme = theme;
                        self.save();
                        // Repaint an open panel immediately; choosing a theme
                        // and seeing nothing happen reads as a broken setting.
                        self.refresh_panel();
                        self.rebuild_menu();
                    }
                }
            }
        }
    }

    fn switch_configuration(
        &mut self,
        name: &str,
        proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    ) {
        let Some(path) = &self.gcloud_path else {
            return;
        };
        if let Err(e) = actions::activate_config(path, name) {
            eprintln!("gcloud-dot: {e}");
            return;
        }
        // A different configuration usually means a different account, so
        // everything known about the old one is now wrong.
        let (config, adc_file) = gcloud_dot_core::read_environment();
        self.engine.set_environment(true, config, adc_file);
        self.reload_configurations();
        self.dispatch_work(
            proxy,
            Plan {
                probe_user: true,
                probe_adc: self.engine.state.settings.track_adc,
                rescan_logs: true,
            },
        );
        self.rebuild_menu();
    }

    fn start_login(&mut self, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
        let Some(path) = self.gcloud_path.clone() else {
            return;
        };
        match actions::login(&path) {
            Ok(mut child) => {
                // Wait off the event loop. A sign in takes as long as the
                // person takes.
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
                    if !ok {
                        let out =
                            std::fs::read_to_string(actions::login_log_path()).unwrap_or_default();
                        let _ =
                            proxy.send_event(UserEvent::LoginFailed(actions::failure_reason(&out)));
                    }
                });
            }
            Err(e) => {
                eprintln!("gcloud-dot: could not start the sign in: {e}");
                return;
            }
        }
        // Poll hard for a few minutes so the dot goes green as soon as the
        // browser hand-off completes, rather than at the next slow tick.
        self.engine.begin_fast_poll(Local::now());
        let _ = proxy.send_event(UserEvent::Tick);
    }

    // ------------------------------------------------------------- panel

    fn toggle_panel(&mut self, target: &tao::event_loop::EventLoopWindowTarget<UserEvent>) {
        if self.panel.is_some() {
            self.close_panel(target);
            return;
        }
        self.open_panel(target);
    }

    fn open_panel(&mut self, target: &tao::event_loop::EventLoopWindowTarget<UserEvent>) {
        let view = panel::view(&self.engine.status, &self.engine.state, &self.update_ui);
        let html = panel::document(&view, self.engine.state.settings.theme);

        let window = match WindowBuilder::new()
            .with_title("GCloud Dot")
            .with_inner_size(tao::dpi::LogicalSize::new(420.0, 760.0))
            // Resizable, with a floor that keeps the two-column detail rows and
            // the pinned action bar from colliding.
            .with_min_inner_size(tao::dpi::LogicalSize::new(340.0, 360.0))
            .with_resizable(true)
            .build(target)
        {
            Ok(w) => w,
            Err(e) => {
                eprintln!("gcloud-dot: could not open the details window: {e}");
                return;
            }
        };

        let proxy = self.proxy.clone();
        let builder = wry::WebViewBuilder::new().with_html(html).with_ipc_handler(
            move |req: wry::http::Request<String>| {
                let _ = proxy.send_event(UserEvent::Panel(req.body().clone()));
            },
        );

        // Attached to the window's GTK box on Linux rather than to the window.
        //
        // `build` is documented as X11 only, and what it produced here was a
        // window that opened, sized itself correctly, ran a WebKitWebProcess,
        // and drew nothing at all: the page background appeared and no element
        // on top of it ever did. The same HTML in a plain WebKit2 view on the
        // same machine rendered perfectly, which is what ruled out the styling
        // and pointed here. wry's own examples all take this route.
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            match window.default_vbox() {
                Some(vbox) => builder.build_gtk(vbox),
                // tao puts that box there itself, so this is unreachable in
                // practice. Falling back rather than failing outright means a
                // future tao that stops doing so costs the window's looks and
                // not the window.
                None => builder.build(&window),
            }
        };
        #[cfg(not(target_os = "linux"))]
        let webview = builder.build(&window);

        match webview {
            Ok(webview) => {
                // A window nobody can reach with the app switcher is a window
                // people lose. macOS hides accessory apps from Cmd-Tab and the
                // Dock, so become a regular app for as long as this is open and
                // step back out when it closes, the Dock icon appears only
                // while there is something to switch to. Windows and Linux put
                // any real window in Alt-Tab already.
                set_app_switcher_visible(target, true);
                window.set_focus();
                self.panel = Some((window, webview));
            }
            Err(e) => eprintln!("gcloud-dot: could not create the details webview: {e}"),
        }
    }

    fn close_panel(&mut self, target: &tao::event_loop::EventLoopWindowTarget<UserEvent>) {
        self.panel = None;
        set_app_switcher_visible(target, false);
    }

    /// Re-render the open panel in place, so a probe finishing while it is open
    /// updates it rather than leaving a stale number on screen.
    fn refresh_panel(&mut self) {
        let Some((_, webview)) = &self.panel else {
            return;
        };
        let view = panel::view(&self.engine.status, &self.engine.state, &self.update_ui);
        // Swapping the content rather than reloading avoids a visible flash and
        // keeps the window's scroll position meaningful.
        let script = panel::refresh_script(&view, self.engine.state.settings.theme);
        let _ = webview.evaluate_script(&script);
    }

    /// Begin an upgrade, unless one is already running.
    ///
    /// The guard is not paranoia: the banner's button and the menu item both
    /// arrive here, and two upgrades writing the same files at once would race
    /// over the staging directory.
    fn start_upgrade(&mut self, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
        if self.update_ui.is_busy() {
            return;
        }
        self.update_ui = update::UpdateUi::Working("Checking for a new version".into());
        self.refresh_panel();

        let progress_proxy = proxy.clone();
        let done_proxy = proxy.clone();
        update::run_in_background(
            move |step| {
                let _ = progress_proxy.send_event(UserEvent::UpdateProgress(step));
            },
            move |result| {
                let _ = done_proxy.send_event(UserEvent::UpdateDone(Box::new(result)));
            },
        );
    }

    fn on_panel_message(
        &mut self,
        message: &str,
        target: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
        proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    ) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(message) else {
            return;
        };
        match value.get("action").and_then(|a| a.as_str()) {
            Some("login") => self.start_login(proxy),
            Some("check") => {
                self.engine.begin_fast_poll(Local::now());
                let plan = Plan {
                    probe_user: true,
                    probe_adc: self.engine.state.settings.track_adc,
                    rescan_logs: true,
                };
                self.dispatch_work(proxy, plan);
            }
            Some("website") => {
                let _ = actions::open_url("https://nicglazkov.github.io/gcloud-dot/");
            }
            Some("update") => self.start_upgrade(proxy),
            // What the release actually contains, for deciding whether to
            // install it now or later.
            Some("notes") => {
                let _ = actions::open_url(update::RELEASES_PAGE);
            }
            // Hides the window and leaves everything else running. This is
            // what most people mean when they reach for a button to dismiss a
            // panel, and before it existed the nearest thing to hand was Quit.
            Some("close") => self.close_panel(target),
            // Routed through the menu handler so quitting behaves identically
            // however it was asked for.
            Some("quit") => {
                let _ = proxy.send_event(UserEvent::Menu(menu::id::QUIT.to_string()));
            }
            _ => {}
        }
    }
}

/// Shown when the tray cannot be created, which on Linux almost always means
/// GNOME without an AppIndicator extension rather than a real failure.
fn linux_tray_hint() -> String {
    if cfg!(all(unix, not(target_os = "macos"))) {
        "No system tray was available. GNOME has not shipped one since 3.26; install the \
         AppIndicator extension (https://extensions.gnome.org/extension/615/appindicator-support/) \
         and log back in. Meanwhile `gcloud-dot status` reports the same information."
            .to_string()
    } else {
        String::new()
    }
}

/// The text to show beside the menu bar icon.
///
/// Empty means "nothing beside the icon", and the caller must still send it, so
/// that a title left over from a previous state is cleared.
///
/// Only a real countdown earns the space. "!" beside a red dot and "?" beside a
/// grey one repeat what the colour already said, and every character is taken
/// from the menu bar's fixed width, permanently, on a notched Mac.
fn menu_bar_title(
    status: &gcloud_dot_core::Status,
    settings: &gcloud_dot_core::Settings,
    now: DateTime<Local>,
) -> String {
    // The platform check is separate from the policy so the policy can be
    // tested on every platform rather than only where the slot exists.
    if cfg!(target_os = "macos") && wants_countdown_text(status, settings, now) {
        status.icon_label(now)
    } else {
        String::new()
    }
}

/// Whether a countdown is worth showing beside the icon at all.
fn wants_countdown_text(
    status: &gcloud_dot_core::Status,
    settings: &gcloud_dot_core::Settings,
    now: DateTime<Local>,
) -> bool {
    settings.show_countdown_text
        && status.auth == gcloud_dot_core::AuthState::Valid
        && status.remaining(now).is_some()
}

/// Show or hide the app in the app switcher and the Dock.
///
/// macOS only: an accessory app has no Dock tile and no Cmd-Tab entry, which is
/// right for a menu bar app with no windows and wrong the moment it opens one.
/// Every other platform lists any real window in its switcher already.
fn set_app_switcher_visible(
    _target: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
    _visible: bool,
) {
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopWindowTargetExtMacOS};
        _target.set_activation_policy_at_runtime(if _visible {
            ActivationPolicy::Regular
        } else {
            ActivationPolicy::Accessory
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcloud_dot_core::estimate::{Estimate, EstimateSource};
    use gcloud_dot_core::{AuthState, Settings, Status};

    fn valid_with(hours_ago: f64) -> Status {
        Status {
            gcloud_found: true,
            auth: AuthState::Valid,
            session_start: Some(
                Local::now() - chrono::Duration::seconds((hours_ago * 3600.0) as i64),
            ),
            estimate: Estimate {
                hours: 16.0,
                source: EstimateSource::Observed { count: 3 },
            },
            ..Default::default()
        }
    }

    #[test]
    fn an_expired_session_wants_no_countdown() {
        // The bug this guards: the session ran past its estimate so the title
        // read "0m", then it expired and the title was never cleared, leaving a
        // red dot beside a stale countdown for as long as the app ran.
        let mut s = valid_with(20.0);
        let settings = Settings::default();
        assert!(wants_countdown_text(&s, &settings, Local::now()));
        assert_eq!(s.icon_label(Local::now()), "0m");

        s.auth = AuthState::Expired;
        assert!(
            !wants_countdown_text(&s, &settings, Local::now()),
            "an expired session must clear the countdown, not keep the last one"
        );
    }

    #[test]
    fn an_unknown_state_wants_no_countdown() {
        let mut s = valid_with(1.0);
        s.auth = AuthState::Unknown("network".into());
        assert!(!wants_countdown_text(
            &s,
            &Settings::default(),
            Local::now()
        ));
    }

    #[test]
    fn a_running_countdown_is_wanted() {
        assert!(wants_countdown_text(
            &valid_with(2.0),
            &Settings::default(),
            Local::now()
        ));
    }

    #[test]
    fn turning_the_countdown_off_wants_nothing() {
        let settings = Settings {
            show_countdown_text: false,
            ..Default::default()
        };
        assert!(!wants_countdown_text(
            &valid_with(2.0),
            &settings,
            Local::now()
        ));
    }

    #[test]
    fn only_macos_has_a_slot_for_the_text() {
        // Windows and Linux draw the countdown into the icon bitmap instead, so
        // the title is always empty there and there is nothing to go stale.
        let title = menu_bar_title(&valid_with(2.0), &Settings::default(), Local::now());
        if cfg!(target_os = "macos") {
            assert!(
                title.ends_with('h'),
                "expected hours remaining, got {title:?}"
            );
        } else {
            assert_eq!(title, "");
        }
    }

    #[test]
    fn the_title_is_empty_once_the_session_is_gone() {
        let mut s = valid_with(20.0);
        s.auth = AuthState::Expired;
        assert_eq!(menu_bar_title(&s, &Settings::default(), Local::now()), "");
    }
}
