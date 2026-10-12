mod action_trigger;
mod audio_output;
mod brightness;
mod desktop;
mod device_session;
mod dial_stack;
mod feedback;
mod hardware;
mod host;
mod instance;
mod layout;
mod multi;
mod multi_editor;
mod package;
mod preferences;
mod profile;
mod render;
mod resources;
mod screensaver;
mod session_activity;
mod soundboard;
mod soundboard_editor;
mod stack_editor;
mod startup;
mod system_actions;
mod system_editor;
mod system_keyboard;
mod tray;
mod trigger_editor;
mod volume;
mod webview;
mod window;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(version, about = "Independent Rust Linux host for Stream Deck+")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Internal native HTML runtime. Launched and supervised by the host.
    #[command(hide = true)]
    Webview {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        port: u16,
        #[arg(long)]
        uuid: String,
        #[arg(long)]
        register_event: String,
        #[arg(long)]
        info: String,
        #[arg(long)]
        action_info: Option<String>,
        #[arg(long)]
        visible: bool,
    },
    /// Open the native Rust desktop application. This is also the default.
    Desktop(DesktopArgs),
    /// Control the existing native application for this state directory.
    Window {
        #[arg(value_parser = ["show", "hide", "close", "quit", "status"])]
        command: String,
        #[arg(long)]
        state: Option<PathBuf>,
    },
    /// Configure login startup for this user. No change occurs until enable/disable.
    Startup {
        #[arg(value_parser = ["enable", "disable", "status"])]
        command: String,
        #[arg(long)]
        state: Option<PathBuf>,
        #[arg(long)]
        simulate: bool,
    },
    /// Enumerate Stream Deck+ devices without changing them.
    Devices,
    /// Read firmware and input events; never changes displays.
    Observe {
        #[arg(long)]
        serial: Option<String>,
        #[arg(long, default_value_t = 30)]
        seconds: u64,
    },
    /// Inspect an Elgato profile archive or directory without executing actions.
    Profile { path: PathBuf },
    /// Validate and unpack a plugin package without executing it.
    Install {
        path: PathBuf,
        #[arg(long, default_value = "state/plugins")]
        destination: PathBuf,
    },
    /// Run the hardware and plugin host with a local browser editor.
    Serve {
        #[arg(long)]
        serial: Option<String>,
        #[arg(long)]
        plugin: Vec<PathBuf>,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long, default_value = "state")]
        state: PathBuf,
        #[arg(long, default_value_t = 17321)]
        port: u16,
        /// Run the real plugin host without opening any USB device.
        #[arg(long)]
        simulate: bool,
        /// Stop after this many seconds, or run until interrupted when omitted.
        #[arg(long)]
        seconds: Option<u64>,
    },
}

#[derive(clap::Args, Default)]
struct DesktopArgs {
    /// Start with the editor hidden; reopen it from the launcher or tray.
    #[arg(long)]
    background: bool,
    #[arg(long)]
    serial: Option<String>,
    #[arg(long)]
    plugin: Vec<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
    #[arg(long)]
    state: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long)]
    simulate: bool,
}
fn desktop_state() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("deckforge")
}
fn main() -> Result<()> {
    match Args::parse()
        .command
        .unwrap_or(Command::Desktop(DesktopArgs::default()))
    {
        Command::Webview {
            path,
            port,
            uuid,
            register_event,
            info,
            action_info,
            visible,
        } => {
            webview::run(webview::WebviewOptions {
                path,
                port,
                uuid,
                register_event,
                info,
                action_info,
                visible,
            })?;
        }
        Command::Window { command, state } => {
            let command = serde_json::from_value(serde_json::Value::String(command))?;
            let response = instance::request(&state.unwrap_or_else(desktop_state), command)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No native Deckforge instance is running for this state directory"
                    )
                })?;
            println!("{}", serde_json::to_string(&response)?);
        }
        Command::Startup {
            command,
            state,
            simulate,
        } => {
            let startup =
                startup::Startup::current(&state.unwrap_or_else(desktop_state), simulate)?;
            let status = match command.as_str() {
                "enable" => startup.set_enabled(true)?,
                "disable" => startup.set_enabled(false)?,
                _ => startup.status()?,
            };
            println!("{}", serde_json::to_string(&status)?);
        }
        Command::Desktop(options) => {
            let state = options.state.unwrap_or_else(desktop_state);
            std::fs::create_dir_all(&state)?;
            let activation = if options.background {
                instance::Command::Status
            } else {
                instance::Command::Show
            };
            if instance::request(&state, activation)?.is_some() {
                println!("Existing Deckforge instance activated");
                return Ok(());
            }
            let running = match host::start(host::HostOptions {
                serial: options.serial,
                plugins: options.plugin,
                profile: options.profile,
                state_dir: state.clone(),
                port: options.port,
                simulate: options.simulate,
                seconds: None,
            }) {
                Ok(running) => running,
                Err(error) => {
                    // A simultaneous launcher can own the session lock before
                    // its window endpoint is ready. Never remove that lock.
                    for _ in 0..10 {
                        std::thread::sleep(Duration::from_millis(100));
                        if instance::request(&state, activation)?.is_some() {
                            println!("Existing Deckforge instance activated");
                            return Ok(());
                        }
                    }
                    return Err(error);
                }
            };
            desktop::run(running.controller.clone(), &state, options.background)?;
            drop(running);
        }
        Command::Devices => {
            for d in hardware::Deck::enumerate()? {
                println!(
                    "{} · {} · {}",
                    d.product.as_deref().unwrap_or("Stream Deck +"),
                    d.serial.as_deref().unwrap_or("unknown serial"),
                    d.path
                );
            }
        }
        Command::Observe { serial, seconds } => {
            let mut deck = hardware::Deck::connect(serial.as_deref())?;
            println!(
                "Stream Deck+ {} firmware {}",
                deck.serial(),
                deck.firmware_version()?
            );
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                for event in deck.read_events(100)? {
                    println!("{event:?}");
                }
            }
        }
        Command::Profile { path } => {
            let p = profile::load(&path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"name":p.name,"uuid":p.uuid,"page":p.page,"pages":p.pages,"slots":p.slots,"warnings":p.warnings})
                )?
            );
        }
        Command::Install { path, destination } => {
            println!(
                "Installed {}",
                package::install(&path, &destination)?.display()
            );
        }
        Command::Serve {
            serial,
            plugin,
            profile,
            state,
            port,
            simulate,
            seconds,
        } => {
            tokio::runtime::Runtime::new()?.block_on(host::serve(
                serial, plugin, profile, state, port, simulate, seconds,
            ))?;
        }
    }
    Ok(())
}
