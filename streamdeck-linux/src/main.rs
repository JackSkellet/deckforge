mod action_defaults;
mod action_icons;
mod action_library;
mod action_trigger;
mod action_wheel;
mod appearance;
mod application_artwork;
mod application_monitor;
mod audio_output;
mod background_movie;
mod brightness;
mod bundled_plugins;
mod deep_link;
mod desktop;
mod desktop_action_file;
mod desktop_file_drop;
mod desktop_web;
mod device_model;
mod device_registry;
mod device_session;
mod dial_stack;
mod external_url;
mod feedback;
mod file_open;
mod focus_monitor;
mod folder_countdown;
mod hardware;
mod host;
mod icon_pack;
mod icon_pack_watch;
mod image_crop;
mod instance;
mod key_animation;
mod key_logic;
mod keyboard_layout;
mod layout;
mod marketplace;
mod mixer;
mod multi;
mod multi_editor;
mod package;
mod plugin_entry;
mod plugin_localization;
mod plugin_update;
mod preferences;
mod profile;
mod protected_manifest;
mod reference_import;
mod render;
mod resources;
mod screensaver;
mod session_activity;
mod shared_action;
mod soundboard;
mod soundboard_editor;
mod stack_editor;
mod startup;
mod system_actions;
mod system_app;
mod system_app_window;
mod system_clipboard;
mod system_close;
mod system_command;
mod system_dial;
mod system_editor;
mod system_keyboard;
mod system_mouse;
mod system_process;
mod system_resume;
mod system_typing;
mod system_x11_window;
mod tray;
mod trigger_editor;
mod volume;
mod webview;
mod webview_assets;
mod webview_files;
mod wheel_animation;
mod window;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(version, about = "Independent Rust Linux host for Stream Deck")]
struct Args {
    /// Plugin callback or local Stream Deck action/profile opened by the desktop.
    #[arg(value_name = "URI_OR_FILE")]
    uri: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect, install or list inert icon packs without launching the host.
    IconPack {
        #[arg(value_parser = ["inspect", "install", "list"])]
        operation: String,
        path: PathBuf,
        #[arg(long)]
        destination: Option<PathBuf>,
        #[arg(long)]
        reference_app: Option<PathBuf>,
    },
    /// Isolated Marketplace session browser. Only the host starts this helper.
    #[command(hide = true)]
    MarketplaceBrowser {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long, value_parser = ["login", "check", "install"])]
        operation: String,
        #[arg(long)]
        variant: Option<uuid::Uuid>,
    },
    /// Internal native HTML runtime. Launched and supervised by the host.
    #[command(hide = true)]
    Webview {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        root: Option<PathBuf>,
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
    /// Read-only focus observer diagnostics; does not switch profiles or windows.
    #[command(hide = true)]
    FocusProbe {
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=30))]
        seconds: u64,
        #[arg(long)]
        events: bool,
    },
    /// Enumerate supported Stream Deck devices without changing them.
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
        /// Local official executable used to verify protected plugin manifests.
        #[arg(long)]
        reference_app: Option<PathBuf>,
    },
    /// Run the hardware and plugin host with a local browser editor.
    Serve {
        /// Restrict hardware selection, or choose an offline/simulated device layout.
        #[arg(long, value_parser = parse_model)]
        model: Option<device_model::Model>,
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
    /// Open a local Stream Deck action, profile, plugin or icon-pack path/file URI.
    #[arg(long, conflicts_with = "uri")]
    open: Option<String>,
    /// Deliver a streamdeck:// plugin callback to this application state.
    #[arg(long)]
    uri: Option<String>,
    /// Restrict hardware selection, or choose an offline/simulated device layout.
    #[arg(long, value_parser = parse_model)]
    model: Option<device_model::Model>,
    /// Use the previous egui editor while the TypeScript migration is validated.
    #[arg(long)]
    legacy_editor: bool,
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
fn parse_model(value: &str) -> Result<device_model::Model, String> {
    device_model::Model::all()
        .iter()
        .copied()
        .find(|model| model.id() == value)
        .ok_or_else(|| {
            format!(
                "unknown model {value}; choose {}",
                device_model::Model::all()
                    .iter()
                    .map(|model| model.id())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn desktop_state() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("deckforge")
}
fn activate_instance(
    state: &std::path::Path,
    activation: instance::Command,
    uri: Option<&str>,
    file: Option<&std::path::Path>,
) -> Result<bool> {
    if let Some(file) = file {
        return Ok(instance::request_open_file(state, file)?.is_some());
    }
    Ok(match uri {
        Some(uri) => instance::request_deep_link(state, uri)?,
        None => instance::request(state, activation)?,
    }
    .is_some())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let command = match (args.command, args.uri) {
        (None, uri) => Command::Desktop(DesktopArgs {
            uri,
            ..Default::default()
        }),
        (Some(command), None) => command,
        (Some(_), Some(_)) => anyhow::bail!("Use a standalone plugin URI or desktop --uri"),
    };
    match command {
        Command::IconPack {
            operation,
            path,
            destination,
            reference_app,
        } => {
            let value = match operation.as_str() {
                "inspect" => {
                    serde_json::to_value(icon_pack::load(&path, reference_app.as_deref())?)?
                }
                "install" => serde_json::to_value(icon_pack::install(
                    &path,
                    &destination
                        .ok_or_else(|| anyhow::anyhow!("--destination is required for install"))?,
                    reference_app.as_deref(),
                )?)?,
                "list" => {
                    serde_json::to_value(icon_pack::catalog(&path, reference_app.as_deref())?)?
                }
                _ => unreachable!(),
            };
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Command::MarketplaceBrowser {
            profile,
            operation,
            variant,
        } => {
            marketplace::browser::run(profile, operation, variant)?;
        }
        Command::Webview {
            path,
            root,
            port,
            uuid,
            register_event,
            info,
            action_info,
            visible,
        } => {
            webview::run(webview::WebviewOptions {
                path,
                root,
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
        Command::Desktop(mut options) => {
            if options.uri.as_ref().is_some_and(|s| {
                !s.split_once("://")
                    .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("streamdeck"))
            }) {
                options.open = options.uri.take();
            }
            let file = options.open.as_deref().map(file_open::parse).transpose()?;
            if file.is_some() {
                options.background = false;
            }
            anyhow::ensure!(
                file.is_none() || !options.legacy_editor,
                "Desktop file opening requires the native TypeScript editor"
            );
            if let Some(uri) = &options.uri {
                let link = deep_link::DeepLink::parse(uri)?;
                anyhow::ensure!(
                    !options.legacy_editor,
                    "Plugin deep links require the native TypeScript editor"
                );
                options.background |= link.hidden;
            }
            let state = options.state.unwrap_or_else(desktop_state);
            std::fs::create_dir_all(&state)?;
            let activation = if options.background {
                instance::Command::Status
            } else {
                instance::Command::Show
            };
            if activate_instance(&state, activation, options.uri.as_deref(), file.as_deref())? {
                println!("Existing Deckforge instance activated");
                return Ok(());
            }
            let running = match host::start(host::HostOptions {
                serial: options.serial,
                model: options.model,
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
                        if activate_instance(
                            &state,
                            activation,
                            options.uri.as_deref(),
                            file.as_deref(),
                        )? {
                            println!("Existing Deckforge instance activated");
                            return Ok(());
                        }
                    }
                    return Err(error);
                }
            };
            if let Some(uri) = &options.uri
                && running.controller.deliver_deep_link(uri).is_err()
            {
                // The controller exposes a sanitized error in the native
                // status/activity UI. Keep that UI available for recovery.
                eprintln!("Plugin callback could not be delivered; see the application status");
            }
            if options.legacy_editor
                && let Some(path) = &file
            {
                let _ = running.controller.open_desktop_file(path);
            }
            if options.legacy_editor {
                running.controller.smart_editor_visible(true);
                anyhow::ensure!(
                    running.controller.snapshot()["device_model"] == "plus",
                    "The legacy editor supports Plus only; launch without --legacy-editor for other models"
                );
                desktop::run(running.controller.clone(), &state, options.background)?;
            } else {
                desktop_web::run(
                    running.controller.clone(),
                    &state,
                    options.background,
                    include_str!("../frontend/dist/index.html"),
                    file.as_deref(),
                )?;
            }
            drop(running);
        }
        Command::FocusProbe { seconds, events } => {
            use std::io::Write;
            let observer = focus_monitor::Worker::new();
            let name = observer.script_name().to_owned();
            let end = Instant::now() + Duration::from_secs(seconds);
            let mut revision = None;
            while Instant::now() < end {
                let snapshot = observer.snapshot();
                if events && revision != Some(snapshot.revision) {
                    println!(
                        "{}",
                        serde_json::to_string(
                            &serde_json::json!({"script":name,"snapshot":snapshot})
                        )?
                    );
                    std::io::stdout().flush()?;
                    revision = Some(snapshot.revision);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let snapshot = observer.snapshot();
            drop(observer);
            let report = serde_json::json!({"script":name,"snapshot":snapshot,"stopped":true});
            println!(
                "{}",
                if events {
                    serde_json::to_string(&report)?
                } else {
                    serde_json::to_string_pretty(&report)?
                }
            );
            anyhow::ensure!(snapshot.error.is_none(), "Focus observer is unavailable");
        }
        Command::Devices => {
            for d in hardware::Deck::enumerate()? {
                println!(
                    "{} · {} · {:04x}:{:04x} · {}",
                    d.product.as_deref().unwrap_or(d.model.name()),
                    d.serial.as_deref().unwrap_or("unknown serial"),
                    d.vendor_id,
                    d.product_id,
                    d.path
                );
            }
        }
        Command::Observe { serial, seconds } => {
            let mut deck = hardware::Deck::connect(serial.as_deref())?;
            println!(
                "{} {} firmware {}",
                deck.model().name(),
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
        Command::Install {
            path,
            destination,
            reference_app,
        } => {
            let installed = if let Some(reference) = reference_app {
                package::install_with_reference(&path, &destination, Some(&reference))?
            } else {
                package::install(&path, &destination)?
            };
            println!("Installed {}", installed.display());
        }
        Command::Serve {
            model,
            serial,
            plugin,
            profile,
            state,
            port,
            simulate,
            seconds,
        } => {
            tokio::runtime::Runtime::new()?.block_on(host::serve(
                serial, plugin, profile, state, port, simulate, seconds, model,
            ))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn desktop_file_entry_accepts_standalone_uri_and_isolated_open_option() {
        let file = "file:///tmp/shared%20action.streamDeckAction";
        assert_eq!(
            Args::try_parse_from(["deckforge", file])
                .unwrap()
                .uri
                .as_deref(),
            Some(file)
        );
        let args = Args::try_parse_from([
            "deckforge",
            "desktop",
            "--open",
            file,
            "--state",
            "/tmp/private-state",
            "--simulate",
        ])
        .unwrap();
        let Some(Command::Desktop(options)) = args.command else {
            panic!("desktop expected")
        };
        assert_eq!(options.open.as_deref(), Some(file));
        assert!(options.simulate);
        assert!(
            Args::try_parse_from([
                "deckforge",
                "desktop",
                "--open",
                file,
                "--uri",
                "streamdeck://plugins/message/example/x"
            ])
            .is_err()
        );
    }

    #[test]
    fn desktop_scheme_handler_accepts_bare_uri_and_explicit_isolated_options() {
        let uri =
            "streamdeck://plugins/message/com.example.test/callback?code=%2F&streamdeck=Hidden";
        let args = Args::try_parse_from(["deckforge", uri]).unwrap();
        assert_eq!(args.uri.as_deref(), Some(uri));
        assert!(args.command.is_none());
        let args = Args::try_parse_from([
            "deckforge",
            "desktop",
            "--uri",
            uri,
            "--simulate",
            "--state",
            "/tmp/owned-test",
        ])
        .unwrap();
        assert!(args.uri.is_none());
        match args.command.unwrap() {
            Command::Desktop(options) => {
                assert_eq!(options.uri.as_deref(), Some(uri));
                assert!(options.simulate);
                assert_eq!(options.state.unwrap(), PathBuf::from("/tmp/owned-test"));
            }
            _ => panic!("Expected desktop command"),
        }
        let args = Args::try_parse_from(["deckforge"]).unwrap();
        assert!(args.command.is_none() && args.uri.is_none());
        assert!(matches!(
            Args::try_parse_from(["deckforge", "devices"])
                .unwrap()
                .command,
            Some(Command::Devices)
        ));
    }
}
