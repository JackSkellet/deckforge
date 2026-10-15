//! Offer-scoped, bounded, memory-only MIME capture. Tested before restore integration.
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{self, Read},
    os::fd::{AsFd, AsRawFd, FromRawFd, RawFd},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    backend::ObjectId,
    protocol::{wl_callback, wl_registry, wl_seat},
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1 as ed, ext_data_control_manager_v1 as em,
    ext_data_control_offer_v1 as eo,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as wd, zwlr_data_control_manager_v1 as wm,
    zwlr_data_control_offer_v1 as wo,
};

const MAX_FORMATS: usize = 32;
const MAX_BYTES: usize = 16 * 1024 * 1024;
pub type Bundle = BTreeMap<String, Vec<u8>>;
#[derive(Clone)]
enum Offer {
    Ext(eo::ExtDataControlOfferV1),
    Wlr(wo::ZwlrDataControlOfferV1),
}
impl Offer {
    fn id(&self) -> ObjectId {
        match self {
            Self::Ext(o) => o.id(),
            Self::Wlr(o) => o.id(),
        }
    }
    fn destroy(&self) {
        match self {
            Self::Ext(o) => o.destroy(),
            Self::Wlr(o) => o.destroy(),
        }
    }
    fn receive(&self, mime: String, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            Self::Ext(o) => o.receive(mime, fd),
            Self::Wlr(o) => o.receive(mime, fd),
        }
    }
}
#[derive(Default)]
struct OfferInfo {
    formats: Vec<String>,
    invalid: bool,
}
#[derive(Default)]
struct State {
    globals: Vec<(u32, String, u32)>,
    offers: HashMap<ObjectId, OfferInfo>,
    selected: Option<Offer>,
    generation: u64,
    seen: bool,
    sync: u32,
    error: Option<String>,
}
impl State {
    fn select(&mut self, offer: Option<Offer>) {
        if let Some(old) = self.selected.take() {
            self.offers.remove(&old.id());
            old.destroy();
        }
        self.selected = offer;
        self.generation += 1;
        self.seen = true;
    }
    fn mime(&mut self, id: ObjectId, mime: String) {
        let offer = self.offers.entry(id).or_default();
        if offer.formats.contains(&mime) {
            return;
        }
        if mime.len() > 256 || mime.is_empty() || offer.formats.len() >= MAX_FORMATS {
            offer.invalid = true;
        } else {
            offer.formats.push(mime);
        }
    }
    fn ignore(&mut self, offer: Option<Offer>) {
        if let Some(offer) = offer {
            self.offers.remove(&offer.id());
            offer.destroy();
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => s.globals.push((name, interface, version)),
            wl_registry::Event::GlobalRemove { name }
                if s.globals.iter().any(|(id, interface, _)| {
                    *id == name
                        && matches!(
                            interface.as_str(),
                            "wl_seat"
                                | "ext_data_control_manager_v1"
                                | "zwlr_data_control_manager_v1"
                        )
                }) =>
            {
                s.error = Some("Clipboard seat or manager disappeared".into())
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, u32> for State {
    fn event(
        s: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        n: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        s.sync = *n;
    }
}
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State: ignore em::ExtDataControlManagerV1);
wayland_client::delegate_noop!(State: ignore wm::ZwlrDataControlManagerV1);
macro_rules! device {
 ($ty:ty,$event:path,$offer:ty,$variant:ident) => {
  impl Dispatch<$ty,()> for State {
   fn event(s:&mut Self,_:&$ty,e:<$ty as Proxy>::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
    use $event as E;
    match e {
     E::DataOffer{id}=>{s.offers.insert(id.id(),OfferInfo::default());},
     E::Selection{id}=>s.select(id.map(Offer::$variant)),
     E::PrimarySelection{id}=>s.ignore(id.map(Offer::$variant)),
     E::Finished=>s.error=Some("Clipboard manager disconnected".into()),
     _=>{}
    }
   }
   wayland_client::event_created_child!(State,$ty,[0=>($offer,())]);
  }
 }
}
device!(
    ed::ExtDataControlDeviceV1,
    ed::Event,
    eo::ExtDataControlOfferV1,
    Ext
);
device!(
    wd::ZwlrDataControlDeviceV1,
    wd::Event,
    wo::ZwlrDataControlOfferV1,
    Wlr
);
macro_rules! offer {
    ($ty:ty,$event:path) => {
        impl Dispatch<$ty, ()> for State {
            fn event(
                s: &mut Self,
                p: &$ty,
                e: <$ty as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                use $event as E;
                if let E::Offer { mime_type } = e {
                    s.mime(p.id(), mime_type);
                }
            }
        }
    };
}
offer!(eo::ExtDataControlOfferV1, eo::Event);
offer!(wo::ZwlrDataControlOfferV1, wo::Event);

pub struct Clipboard {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    sequence: u32,
    cancel: Arc<AtomicBool>,
    protocol: &'static str,
}
impl Clipboard {
    pub fn connect(cancel: Arc<AtomicBool>) -> Result<Self> {
        Self::connect_protocol(cancel, true)
    }
    pub fn connect_protocol(cancel: Arc<AtomicBool>, prefer_ext: bool) -> Result<Self> {
        ensure!(
            std::env::var_os("WAYLAND_SOCKET").is_none(),
            "Inherited Wayland FDs are not supported for clipboard capture"
        );
        let display = std::env::var_os("WAYLAND_DISPLAY").context("Wayland display unavailable")?;
        let display = PathBuf::from(display);
        let path = if display.is_absolute() {
            display
        } else {
            PathBuf::from(
                std::env::var_os("XDG_RUNTIME_DIR").context("Wayland runtime unavailable")?,
            )
            .join(display)
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let socket = runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    tokio::net::UnixStream::connect(path),
                )
                .await
            })??
            .into_std()?;
        let connection = Connection::from_socket(socket)?;
        let queue = connection.new_event_queue();
        let q = queue.handle();
        let registry = connection.display().get_registry(&q, ());
        let mut this = Self {
            connection,
            queue,
            state: State::default(),
            sequence: 0,
            cancel,
            protocol: "pending",
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        this.sync(deadline)?;
        let seats: Vec<_> = this
            .state
            .globals
            .iter()
            .filter(|(_, i, _)| i == "wl_seat")
            .collect();
        ensure!(
            seats.len() == 1,
            "Clipboard capture requires one unambiguous seat"
        );
        let seat: wl_seat::WlSeat = registry.bind(seats[0].0, seats[0].2.min(4), &q, ());
        if let Some((id, _, _)) = this
            .state
            .globals
            .iter()
            .find(|(_, i, _)| prefer_ext && i == "ext_data_control_manager_v1")
        {
            this.protocol = "ext_data_control_manager_v1";
            let manager: em::ExtDataControlManagerV1 = registry.bind(*id, 1, &q, ());
            manager.get_data_device(&seat, &q, ());
        } else if let Some((id, _, _)) = this
            .state
            .globals
            .iter()
            .find(|(_, i, _)| i == "zwlr_data_control_manager_v1")
        {
            this.protocol = "zwlr_data_control_manager_v1";
            let manager: wm::ZwlrDataControlManagerV1 = registry.bind(*id, 1, &q, ());
            manager.get_data_device(&seat, &q, ());
        } else {
            bail!("Desktop does not provide clipboard data control");
        }
        this.sync(deadline)?;
        ensure!(
            this.state.seen,
            "Desktop did not report its clipboard selection"
        );
        Ok(this)
    }
    fn sync(&mut self, deadline: Instant) -> Result<()> {
        self.sequence += 1;
        self.connection
            .display()
            .sync(&self.queue.handle(), self.sequence);
        while self.state.sync < self.sequence {
            self.pump(deadline, None)?;
        }
        Ok(())
    }
    fn pump(&mut self, deadline: Instant, pipe: Option<RawFd>) -> Result<()> {
        ensure!(
            !self.cancel.load(Ordering::Acquire),
            "Clipboard capture cancelled"
        );
        ensure!(Instant::now() < deadline, "Clipboard capture timed out");
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(error) = &self.state.error {
            bail!("{error}");
        }
        self.connection.flush()?;
        if let Some(guard) = self.queue.prepare_read() {
            let mut fds = [
                libc::pollfd {
                    fd: self.connection.as_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: pipe.unwrap_or(-1),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let milliseconds = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(10) as i32;
            let result =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, milliseconds) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error.into());
                }
            } else if fds[0].revents != 0 {
                guard.read()?;
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(error) = &self.state.error {
            bail!("{error}");
        }
        Ok(())
    }
    pub fn supports_wlr(&self) -> bool {
        self.state
            .globals
            .iter()
            .any(|(_, interface, _)| interface == "zwlr_data_control_manager_v1")
    }
    pub fn protocol(&self) -> &'static str {
        self.protocol
    }
    pub fn capture(&mut self) -> Result<Bundle> {
        self.capture_with(|| {})
    }
    pub fn capture_with(&mut self, mut after_format: impl FnMut()) -> Result<Bundle> {
        let deadline = Instant::now() + Duration::from_secs(3);
        self.sync(deadline)?;
        let generation = self.state.generation;
        let Some(offer) = self.state.selected.clone() else {
            return Ok(Bundle::new());
        };
        let info = self
            .state
            .offers
            .get(&offer.id())
            .context("Clipboard offer metadata missing")?;
        ensure!(!info.invalid, "Clipboard format list exceeds limits");
        let formats = info.formats.clone();
        let mut bundle = Bundle::new();
        let mut total = 0;
        for mime in formats {
            let mut fds = [-1; 2];
            ensure!(
                unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } == 0,
                "Create clipboard receive pipe"
            );
            let mut reader = unsafe { File::from_raw_fd(fds[0]) };
            let writer = unsafe { File::from_raw_fd(fds[1]) };
            // Only our read end is nonblocking; the producer receives its normal pipe end.
            let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
            ensure!(
                flags >= 0
                    && unsafe {
                        libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK)
                    } >= 0,
                "Configure clipboard pipe"
            );
            offer.receive(mime.clone(), writer.as_fd());
            self.connection.flush()?;
            drop(writer);
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                ensure!(
                    !self.cancel.load(Ordering::Acquire),
                    "Clipboard capture cancelled"
                );
                ensure!(Instant::now() < deadline, "Clipboard capture timed out");
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        total += count;
                        ensure!(total <= MAX_BYTES, "Clipboard capture exceeds 16 MiB");
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        self.pump(deadline, Some(reader.as_raw_fd()))?
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e.into()),
                }
                ensure!(
                    self.state.generation == generation,
                    "Clipboard changed while capturing"
                );
            }
            bundle.insert(mime, bytes);
            after_format();
            self.sync(deadline)?;
            ensure!(
                self.state.generation == generation,
                "Clipboard changed while capturing"
            );
        }
        Ok(bundle)
    }
}
