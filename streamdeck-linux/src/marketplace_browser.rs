//! A separate process and browser profile for Elgato's own sign-in/session flow.
use super::{BrowserReply, private_directory};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
use wry::{WebContext, WebViewBuilder};

const HOME: &str = "https://marketplace.elgato.com/account/products";
const LOGIN: &str = "https://marketplace.elgato.com/auth/login?callbackUrl=%2Faccount%2Fproducts";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    nonce: uuid::Uuid,
    reply: BrowserReply,
}

fn parse_reply(origin: &str, body: &str, nonce: uuid::Uuid) -> Option<BrowserReply> {
    if !trusted_origin(origin) || body.len() > 32768 {
        return None;
    }
    let envelope: Envelope = serde_json::from_str(body).ok()?;
    (envelope.nonce == nonce).then_some(envelope.reply)
}

pub fn trusted_origin(uri: &str) -> bool {
    url::Url::parse(uri)
        .is_ok_and(|url| url.origin().ascii_serialization() == "https://marketplace.elgato.com")
}

pub fn run(profile: PathBuf, operation: String, variant: Option<uuid::Uuid>) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        // Do not leave a browser writing cookies after its owning host dies and
        // releases the profile lock. Cover the race while installing the signal.
        let parent = unsafe { libc::getppid() };
        anyhow::ensure!(
            unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) } == 0,
            "Could not bind Marketplace browser to its parent"
        );
        anyhow::ensure!(
            unsafe { libc::getppid() } == parent && parent != 1,
            "Marketplace browser parent exited"
        );
    }
    // The helper owns its process umask. Never alter the multithreaded host's.
    unsafe {
        libc::umask(0o077);
    }
    private_directory(&profile)?;
    anyhow::ensure!(
        operation != "install" || variant.is_some(),
        "Marketplace variant missing"
    );
    let mut event_loop = EventLoopBuilder::<BrowserReply>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Deckforge · Sign in to Elgato Marketplace")
        .with_inner_size(LogicalSize::new(680.0, 800.0))
        .with_visible(operation == "login")
        .build(&event_loop)?;
    let mut context = WebContext::new(Some(profile));
    let proxy = event_loop.create_proxy();
    let nonce = uuid::Uuid::new_v4();
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        // Login can use the identity provider's normal HTTPS redirects. External
        // protocols, downloads and new windows never execute through this helper.
        .with_navigation_handler(|uri| url::Url::parse(&uri).is_ok_and(|u| u.scheme() == "https"))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
        .with_ipc_handler(move |request| {
            // WebKit reports the top document URI even for iframe messages.
            // A per-process capability is supplied only to our top-frame probe.
            if let Some(reply) = parse_reply(&request.uri().to_string(), request.body(), nonce) {
                let _ = proxy.send_event(reply);
            }
        });
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(
            window
                .default_vbox()
                .context("Marketplace window has no GTK container")?,
        )?
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window)?;
    let script = include_str!("marketplace_session.js")
        .replace("__NONCE__", &serde_json::to_string(&nonce)?)
        .replace("__OPERATION__", &serde_json::to_string(&operation)?)
        .replace(
            "__VARIANT__",
            &serde_json::to_string(&variant.map(|id| id.to_string()))?,
        );
    webview.load_url(HOME)?;
    let mut next_probe = Instant::now();
    let mut deadline =
        Instant::now() + Duration::from_secs(if operation == "login" { 600 } else { 60 });
    let mut signing_in = false;
    let mut result = BrowserReply::error("Marketplace sign-in was cancelled. Retry when ready.");
    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::WaitUntil(next_probe);
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            Event::UserEvent(reply)
                if reply.kind == "signed_out" && operation != "check" && !signing_in =>
            {
                signing_in = true;
                deadline = Instant::now() + Duration::from_secs(600);
                window.set_visible(true);
                window.set_focus();
                if webview.load_url(LOGIN).is_err() {
                    result = BrowserReply::error("Could not open Elgato sign-in.");
                    *flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(reply) => {
                result = reply;
                *flow = ControlFlow::Exit;
            }
            Event::MainEventsCleared => {
                let now = Instant::now();
                if now >= deadline {
                    result = BrowserReply::error(
                        "Marketplace did not respond in time. Retry when ready.",
                    );
                    *flow = ControlFlow::Exit;
                } else if now >= next_probe {
                    next_probe = now + Duration::from_secs(1);
                    let _ = webview.evaluate_script(&script);
                }
            }
            _ => {}
        }
    });
    drop(webview);
    drop(context);
    // This is an owned pipe to the host, never a log or the editor bridge. Only
    // the short-lived signed download URL crosses it; cookies remain in WebKit.
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsolicited_frame_messages_need_the_probe_capability() {
        let nonce = uuid::Uuid::new_v4();
        let body =
            serde_json::json!({"nonce":nonce,"reply":{"kind":"authenticated","name":"Account"}})
                .to_string();
        assert!(parse_reply(HOME, &body, nonce).is_some());
        assert!(parse_reply(HOME, &body, uuid::Uuid::new_v4()).is_none());
        assert!(parse_reply("https://account.elgato.com/", &body, nonce).is_none());
        assert!(parse_reply(HOME, r#"{"kind":"authenticated","name":"Forged"}"#, nonce).is_none());
    }
}
