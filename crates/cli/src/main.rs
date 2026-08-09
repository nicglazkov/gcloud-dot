//! `gcloud-dot` — the command line half of GCloud Dot.
//!
//! Everything the tray shows is available here, which is what makes the tool
//! useful on a server with no tray at all and inside a shell prompt.

mod render;
mod snapshot;

use chrono::Local;
use clap::{Parser, Subcommand};
use gcloud_dot_core::{gcloud, paths, status::AuthState};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "gcloud-dot",
    version = gcloud_dot_core::VERSION,
    about = "Shows how long your gcloud auth session has left.",
    long_about = "Shows how long your gcloud auth session has left.\n\n\
                  Whether the session is alive is measured by running gcloud.\n\
                  How long it has left is predicted from sessions already seen\n\
                  ending, and every output says which of the two it is."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Emit JSON instead of text. Stable across releases.
    #[arg(long, global = true)]
    json: bool,

    /// Skip the gcloud call and report only what is already on disk.
    #[arg(long, global = true)]
    offline: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Show the current session status. The default.
    Status,
    /// Force a check right now and report what it found.
    Check,
    /// Run `gcloud auth login`.
    Login,
    /// Show login history and measured session lengths.
    History,
    /// Show the active gcloud configuration, or switch to another.
    Config {
        /// Configuration to activate. Omit to list.
        name: Option<String>,
    },
    /// Print where state and settings are kept.
    Paths,
    /// Ask the running menu bar or tray app to exit.
    Quit,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Status) {
        Command::Status => status(cli.json, !cli.offline),
        Command::Check => status(cli.json, true),
        Command::Login => login(),
        Command::History => history(cli.json),
        Command::Config { name } => config(name, cli.json),
        Command::Paths => paths_cmd(cli.json),
        Command::Quit => quit(),
    }
}

/// Exit codes are part of the interface: a prompt or a CI step can branch on
/// them without parsing anything.
///
/// 0 signed in · 1 signed out · 2 unknown
fn status(json: bool, run_probe: bool) -> ExitCode {
    let snap = snapshot::take(run_probe);
    if json {
        println!("{}", render::status_json(&snap.status, &snap.state));
    } else {
        print!("{}", render::status_text(&snap.status, &snap.state));
    }
    match snap.status.auth {
        AuthState::Valid => ExitCode::SUCCESS,
        AuthState::Expired => ExitCode::from(1),
        AuthState::Unknown(_) => ExitCode::from(2),
    }
}

fn login() -> ExitCode {
    let Some(path) = gcloud::find() else {
        eprintln!("gcloud is not installed, or is not somewhere this tool looked.");
        eprintln!("Install the Google Cloud SDK: https://cloud.google.com/sdk/docs/install");
        return ExitCode::from(2);
    };
    // Inherited stdio on purpose: the browser hand-off prints a URL the user
    // may need to copy, and the account chooser asks questions.
    let status = std::process::Command::new(&path)
        .args(["auth", "login"])
        .status();
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(e) => {
            eprintln!("could not run {}: {e}", path.display());
            ExitCode::from(2)
        }
    }
}

fn history(json: bool) -> ExitCode {
    let snap = snapshot::take(false);
    if json {
        println!("{}", render::history_json(&snap.state));
    } else {
        print!("{}", render::history_text(&snap.state));
    }
    ExitCode::SUCCESS
}

fn config(name: Option<String>, json: bool) -> ExitCode {
    let Some(dir) = paths::gcloud_config_dir() else {
        eprintln!("could not work out where gcloud keeps its configuration");
        return ExitCode::from(2);
    };

    let Some(target) = name else {
        let active = gcloud_dot_core::config::active(&dir);
        let all = gcloud_dot_core::config::list(&dir);
        if json {
            println!("{}", render::config_json(active.as_ref(), &all));
        } else {
            print!("{}", render::config_text(active.as_ref(), &all));
        }
        return ExitCode::SUCCESS;
    };

    let Some(path) = gcloud::find() else {
        eprintln!("gcloud is not installed, so configurations cannot be switched.");
        return ExitCode::from(2);
    };
    // Switching is delegated to gcloud rather than done by writing
    // active_config: gcloud validates the name and keeps its own caches
    // straight, and quietly duplicating that is how state gets corrupted.
    let out = std::process::Command::new(&path)
        .args(["config", "configurations", "activate", &target])
        .status();
    match out {
        Ok(s) if s.success() => {
            println!("switched to {target}");
            ExitCode::SUCCESS
        }
        _ => ExitCode::from(1),
    }
}

/// Ask a running tray to exit, and say honestly whether one was listening.
///
/// This exists because the tray's own Quit item lives in a menu behind an icon,
/// and a full menu bar means macOS never draws that icon. Without a route from
/// the terminal the only way out would be Activity Monitor.
fn quit() -> ExitCode {
    if let Err(e) = gcloud_dot_core::request_quit() {
        eprintln!("could not write the quit request: {e}");
        return ExitCode::from(2);
    }

    // The tray removes the file as it exits, so its disappearance is the
    // acknowledgement. Poll a little longer than one tick.
    let path = gcloud_dot_core::paths::quit_request_path();
    for _ in 0..40 {
        if !path.exists() {
            println!("GCloud Dot is shutting down.");
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    // Nothing consumed it. Clear the request rather than leave a landmine that
    // would make the next launch exit immediately.
    let _ = std::fs::remove_file(&path);
    eprintln!("No running GCloud Dot answered. It may already be stopped.");
    ExitCode::from(1)
}

fn paths_cmd(json: bool) -> ExitCode {
    let state = paths::state_path();
    let gcloud_cfg = paths::gcloud_config_dir();
    let logs = paths::gcloud_log_dir();
    let adc = paths::adc_path();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "state": state,
                "gcloud_config": gcloud_cfg,
                "gcloud_logs": logs,
                "adc": adc,
                "gcloud_binary": gcloud::find(),
            })
        );
    } else {
        println!("state         {}", state.display());
        println!(
            "gcloud config {}",
            gcloud_cfg
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        println!(
            "gcloud logs   {}",
            logs.map(|p| p.display().to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        println!(
            "adc           {}",
            adc.map(|p| p.display().to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        println!(
            "gcloud        {}",
            gcloud::find()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".into())
        );
    }
    let _ = Local::now();
    ExitCode::SUCCESS
}
