mod desktop;
mod hardware;
mod host;
mod package;
mod profile;
mod render;

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
    /// Open the native Rust desktop application. This is also the default.
    Desktop(DesktopArgs),
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
        Command::Desktop(options) => {
            let running = host::start(host::HostOptions {
                serial: options.serial,
                plugins: options.plugin,
                profile: options.profile,
                state_dir: options.state.unwrap_or_else(desktop_state),
                port: options.port,
                simulate: options.simulate,
                seconds: None,
            })?;
            desktop::run(running.controller.clone())?;
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
