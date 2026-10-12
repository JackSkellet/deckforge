//! StatusNotifier tray service; callbacks must enqueue work rather than block.
//!
//! Online means the watcher accepted registration, not that an icon was visibly
//! painted. ksni 0.3.6 exposes no Wayland activation token: Show is a request for
//! the UI to restore its window using its normal platform facilities.
use anyhow::{Context, Result};
use ksni::TrayMethods;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc,
};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{Notify, oneshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Show,
    Quit,
    Online(bool),
    Error(String),
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;
const UNKNOWN: u8 = 0;
const OFFLINE: u8 = 1;
const VERIFY: u8 = 2;
const ONLINE: u8 = 3;

struct Item {
    callback: Callback,
    icon: ksni::Icon,
    availability: Arc<AtomicU8>,
    changed: Arc<Notify>,
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "deckforge".into()
    }
    fn title(&self) -> String {
        "Deckforge".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        (self.callback)(Event::Show);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open Deckforge".into(),
                activate: Box::new(|item: &mut Self| (item.callback)(Event::Show)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|item: &mut Self| (item.callback)(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_online(&self) {
        // ksni invokes this before re-registering. Let its service loop finish
        // registration before announcing availability through an update barrier.
        self.availability.store(VERIFY, Ordering::Release);
        self.changed.notify_one();
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        self.availability.store(OFFLINE, Ordering::Release);
        (self.callback)(Event::Online(false));
        if let ksni::OfflineReason::Error(error) = reason {
            (self.callback)(Event::Error(error.to_string()));
        }
        true // Retain the service so a later watcher can register it.
    }
}

fn icon() -> Result<ksni::Icon> {
    let tree = resvg::usvg::Tree::from_data(
        include_bytes!("../assets/deckforge.svg"),
        &resvg::usvg::Options::default(),
    )
    .context("parse Deckforge tray icon")?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(64, 64).context("allocate tray icon")?;
    let size = tree.size();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(64.0 / size.width(), 64.0 / size.height()),
        &mut pixmap.as_mut(),
    );
    Ok(ksni::Icon {
        width: 64,
        height: 64,
        data: argb(&pixmap),
    })
}

fn argb(pixmap: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    // tiny-skia is premultiplied RGBA; SNI requests straight ARGB in network
    // byte order. Merely swapping bytes would darken translucent edge pixels.
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.alpha(), color.red(), color.green(), color.blue()]
        })
        .collect()
}

pub struct Tray {
    cancel: Option<oneshot::Sender<()>>,
    finished: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

/// Starts on an independent thread; neither session-bus startup nor an absent
/// watcher blocks the UI. Callback implementations must return promptly.
pub fn spawn(callback: Callback) -> Result<Tray> {
    let (cancel, mut stop) = oneshot::channel();
    let (finished_tx, finished) = mpsc::channel();
    let thread = std::thread::Builder::new().name("deckforge-tray".into()).spawn(move || {
        let result = (|| -> Result<()> {
            let icon = icon()?;
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            runtime.block_on(async {
                let availability = Arc::new(AtomicU8::new(UNKNOWN));
                let changed = Arc::new(Notify::new());
                let item = Item { callback: callback.clone(), icon,
                    availability: availability.clone(), changed: changed.clone() };
                let handle = tokio::select! {
                    biased;
                    _ = &mut stop => return,
                    result = item.assume_sni_available(true).spawn() => match result {
                        Ok(handle) => handle,
                        Err(error) => {
                            callback(Event::Online(false));
                            callback(Event::Error(error.to_string()));
                            return;
                        }
                    }
                };
                // A soft startup error already marked OFFLINE. Only a successful
                // initial watcher registration may enter the verification path.
                if availability.compare_exchange(UNKNOWN, VERIFY, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    changed.notify_one();
                }
                let mut check = tokio::time::interval(Duration::from_millis(250));
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut stop => break,
                        _ = changed.notified() => {},
                        _ = check.tick() => {},
                    }
                    if handle.is_closed() {
                        callback(Event::Online(false));
                        callback(Event::Error("Tray service stopped".into()));
                        break;
                    }
                    if availability.load(Ordering::Acquire) == VERIFY {
                        let verified = tokio::select! {
                            biased;
                            _ = &mut stop => break,
                            result = tokio::time::timeout(Duration::from_millis(500), handle.update(|_| ())) =>
                                matches!(result, Ok(Some(()))),
                        };
                        if verified && availability.compare_exchange(VERIFY, ONLINE, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                            callback(Event::Online(true));
                        }
                    }
                }
                let _ = tokio::time::timeout(Duration::from_millis(400), handle.shutdown()).await;
            });
            runtime.shutdown_timeout(Duration::from_millis(100));
            Ok(())
        })();
        if let Err(error) = result {
            callback(Event::Online(false));
            callback(Event::Error(format!("Tray startup failed: {error:#}")));
        }
        let _ = finished_tx.send(());
    }).context("start tray service thread")?;
    Ok(Tray {
        cancel: Some(cancel),
        finished,
        thread: Some(thread),
    })
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        // Startup and async service waits are cancellable. Avoid trapping the UI
        // forever if a caller violates the nonblocking callback contract.
        if self.finished.recv_timeout(Duration::from_secs(1)).is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray as _;

    #[test]
    fn icon_has_network_argb_and_unpremultiplied_edges() {
        let icon = icon().unwrap();
        assert_eq!(
            (icon.width, icon.height, icon.data.len()),
            (64, 64, 64 * 64 * 4)
        );
        assert_eq!(&icon.data[..4], &[0, 0, 0, 0]);
        assert!(icon.data.as_chunks::<4>().0.contains(&[255, 108, 218, 187]));
        let mut pixel = resvg::tiny_skia::Pixmap::new(1, 1).unwrap();
        pixel.data_mut().copy_from_slice(&[64, 32, 16, 128]);
        assert_eq!(argb(&pixel), [128, 128, 64, 32]);
    }

    #[test]
    fn menu_and_activation_emit_distinct_requests() {
        let (tx, rx) = mpsc::channel();
        let mut item = Item {
            callback: Arc::new(move |e| {
                tx.send(e).unwrap();
            }),
            icon: icon().unwrap(),
            availability: Arc::new(AtomicU8::new(UNKNOWN)),
            changed: Arc::new(Notify::new()),
        };
        let menu = item.menu();
        let ksni::MenuItem::Standard(open) = &menu[0] else {
            panic!("missing Open");
        };
        let ksni::MenuItem::Standard(quit) = &menu[2] else {
            panic!("missing Quit");
        };
        assert_eq!(open.label, "Open Deckforge");
        assert_eq!(quit.label, "Quit");
        assert!(open.enabled && open.visible && quit.enabled && quit.visible);
        (open.activate)(&mut item);
        (quit.activate)(&mut item);
        item.activate(10, 20);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [Event::Show, Event::Quit, Event::Show]
        );
        item.watcher_online();
        assert!(rx.try_recv().is_err(), "unverified online callback leaked");
        assert_eq!(item.availability.load(Ordering::Acquire), VERIFY);
        assert!(item.watcher_offline(ksni::OfflineReason::No));
        assert_eq!(rx.try_recv().unwrap(), Event::Online(false));
    }
}
