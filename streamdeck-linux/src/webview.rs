//! An isolated native browser process for a plugin's own HTML.
//!
//! Registration follows Elgato's documented browser callback:
//! https://docs.elgato.com/streamdeck/sdk/v1/references/websocket/ui/
//! The caller authorizes the plugin path and owns this process's lifetime.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::{Theme, WindowBuilder},
};
use url::Url;
use wry::{PageLoadEvent, WebViewBuilder};

pub struct WebviewOptions {
    /// The host must first check that this is under the authorized plugin root.
    pub path: PathBuf,
    pub port: u16,
    pub uuid: String,
    pub register_event: String,
    /// Serialized JSON, passed to JavaScript as a string as required by the SDK.
    pub info: String,
    pub action_info: Option<String>,
    pub visible: bool,
}

enum BrowserEvent {
    PageReady,
    RegistrationResult(String),
}

/// Run on this helper process's main thread. Closing a visible inspector returns
/// normally; the parent terminates a hidden plugin helper when stopping its host.
pub fn run(options: WebviewOptions) -> Result<()> {
    let url = local_html_url(&options.path)?;
    let script = registration_script(&options)?;
    let mut event_loop = EventLoopBuilder::<BrowserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title(if options.visible {
            "Deckforge · Plugin settings"
        } else {
            "Deckforge · HTML plugin"
        })
        .with_theme(Some(Theme::Dark))
        .with_inner_size(LogicalSize::new(600.0, 760.0))
        .with_min_inner_size(LogicalSize::new(320.0, 360.0))
        .with_visible(options.visible)
        .with_focused(options.visible)
        .build(&event_loop)
        .context("Create native plugin browser window")?;

    let load_proxy = event_loop.create_proxy();
    let expected_url = url.clone();
    let builder = WebViewBuilder::new()
        .with_url(url.as_str())
        .with_background_color((19, 23, 29, 255))
        .with_focused(options.visible)
        // Wry creates related native browser windows, preserving window.opener.
        // The bundled tutorial uses this for its local actions/*.html pages.
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Allow)
        .with_on_page_load_handler(move |event, loaded_url| {
            // Never pass the registration identity to a subsequently navigated page.
            if matches!(event, PageLoadEvent::Finished) && same_document(&expected_url, &loaded_url)
            {
                let _ = load_proxy.send_event(BrowserEvent::PageReady);
            }
        });

    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder
            .build_gtk(
                window
                    .default_vbox()
                    .context("Native window has no GTK container")?,
            )
            .context("Create WebKit plugin browser")?
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window).context("Create plugin browser")?;

    #[cfg(target_os = "linux")]
    if options.register_event == "registerPlugin" {
        use webkit2gtk::{SettingsExt, WebViewExt};
        use wry::WebViewExtUnix;
        // A physical deck press arrives over WebSocket, not as a DOM gesture.
        // The authorized HTML backend must still be able to open its own native
        // action windows (the bundled tutorial does this). File-origin rules
        // and the user's ordinary browser preferences remain intact.
        if let Some(settings) = webview.webview().settings() {
            settings.set_javascript_can_open_windows_automatically(true);
        }
    }

    let result_proxy = event_loop.create_proxy();
    let mut invoked = false;
    let mut failure = None;
    let exit_code = event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(BrowserEvent::PageReady) if !invoked => {
                // A document can emit multiple load notifications. Register once.
                invoked = true;
                let proxy = result_proxy.clone();
                if let Err(error) = webview.evaluate_script_with_callback(&script, move |result| {
                    let _ = proxy.send_event(BrowserEvent::RegistrationResult(result));
                }) {
                    failure = Some(format!("Invoke plugin registration callback: {error}"));
                    *flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(BrowserEvent::RegistrationResult(result)) => {
                if let Err(error) = registration_result(&result) {
                    failure = Some(error.to_string());
                    *flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if window_id == window.id() => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    drop(webview);
    drop(window);
    if let Some(error) = failure {
        eprintln!("Plugin browser: {error}");
        bail!(error);
    }
    ensure!(
        exit_code == 0,
        "Plugin browser event loop exited with code {exit_code}"
    );
    Ok(())
}

fn local_html_url(path: &Path) -> Result<Url> {
    let path = path.canonicalize().context("Resolve plugin HTML path")?;
    ensure!(path.is_file(), "Plugin HTML path is not a file");
    Url::from_file_path(&path)
        .map_err(|()| anyhow::anyhow!("Cannot convert plugin HTML path to a file URL"))
}

fn same_document(expected: &Url, loaded: &str) -> bool {
    let Ok(mut loaded) = Url::parse(loaded) else {
        return false;
    };
    loaded.set_fragment(None);
    loaded == *expected
}

fn registration_arguments(options: &WebviewOptions) -> Result<String> {
    ensure!(options.port != 0, "Plugin WebSocket port must not be zero");
    ensure!(
        !options.uuid.is_empty(),
        "Plugin registration identity is missing"
    );
    ensure!(
        matches!(
            options.register_event.as_str(),
            "registerPlugin" | "registerPropertyInspector"
        ),
        "Unknown browser registration event"
    );
    let info: Value =
        serde_json::from_str(&options.info).context("Invalid registration info JSON")?;
    ensure!(info.is_object(), "Registration info must be a JSON object");
    let mut args = vec![
        options.port.to_string(),
        options.uuid.clone(),
        options.register_event.clone(),
        options.info.clone(),
    ];
    if let Some(action_info) = &options.action_info {
        let parsed: Value =
            serde_json::from_str(action_info).context("Invalid action info JSON")?;
        ensure!(parsed.is_object(), "Action info must be a JSON object");
        args.push(action_info.clone());
    }
    // JSON encoding prevents quotes, backslashes, and embedded source-looking
    // strings from changing the JavaScript program. Escape historical JS separators too.
    Ok(serde_json::to_string(&args)?
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

fn registration_script(options: &WebviewOptions) -> Result<String> {
    let args = registration_arguments(options)?;
    Ok(format!(
        r#"(() => {{
        try {{
            const connect = window.connectElgatoStreamDeckSocket;
            if (typeof connect !== "function") {{
                return {{ok: false, error: "Plugin HTML did not define connectElgatoStreamDeckSocket"}};
            }}
            connect.apply(window, {args});
            return {{ok: true}};
        }} catch (error) {{
            return {{ok: false, error: String(error)}};
        }}
    }})()"#
    ))
}

fn registration_result(result: &str) -> Result<()> {
    let value: Value =
        serde_json::from_str(result).context("Browser returned an invalid registration result")?;
    if value["ok"] != true {
        bail!(
            "Plugin registration callback failed: {}",
            value["error"].as_str().unwrap_or("unknown browser error")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> WebviewOptions {
        WebviewOptions {
            path: PathBuf::from("unused.html"),
            port: 12345,
            uuid: "plugin-id".into(),
            register_event: "registerPlugin".into(),
            info: "{\"language\":\"en\"}".into(),
            action_info: None,
            visible: false,
        }
    }

    #[test]
    fn local_paths_round_trip_spaces_unicode_hash_and_percent() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("plugin settings æ #100%.html");
        std::fs::write(&file, "<!doctype html>").unwrap();
        let url = local_html_url(&file).unwrap();
        assert_eq!(url.to_file_path().unwrap(), file.canonicalize().unwrap());
        assert!(url.as_str().contains("%20"));
        assert!(url.as_str().contains("%23"));
        assert!(url.as_str().contains("%25"));
        assert!(same_document(&url, &(url.to_string() + "#settings")));
        assert!(!same_document(&url, "https://example.com/"));
    }

    #[test]
    fn registration_arguments_remain_strings_and_round_trip_source_like_text() {
        let mut options = options();
        options.uuid = "\"); window.unwanted = true; //\n\\\u{2028}".into();
        options.action_info = Some("{\"title\":\"quote \\\" slash \\\\ </script>\"}".into());
        options.register_event = "registerPropertyInspector".into();
        let args: Vec<String> =
            serde_json::from_str(&registration_arguments(&options).unwrap()).unwrap();
        assert_eq!(
            args,
            vec![
                "12345".to_owned(),
                options.uuid.clone(),
                options.register_event.clone(),
                options.info.clone(),
                options.action_info.clone().unwrap()
            ]
        );
        let script = registration_script(&options).unwrap();
        assert!(!script.contains('\u{2028}'));
        assert!(script.contains("connect.apply(window, ["));
        options.action_info = None;
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&registration_arguments(&options).unwrap())
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn invalid_registration_payloads_and_callback_failures_are_errors() {
        let mut options = options();
        options.info = "[]".into();
        assert!(registration_script(&options).is_err());
        options.info = "{}".into();
        options.action_info = Some("invalid".into());
        assert!(registration_script(&options).is_err());
        assert!(
            registration_result("{\"ok\":false,\"error\":\"missing callback\"}")
                .unwrap_err()
                .to_string()
                .contains("missing callback")
        );
        assert!(registration_result("undefined").is_err());
        assert!(registration_result("{\"ok\":true}").is_ok());
    }
}
