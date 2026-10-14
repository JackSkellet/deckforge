//! Local X11 window control: EWMH inventory, authoritative XRes PIDs and ICCCM close.
use crate::{system_app_window::CloseResult, system_process::PinnedProcess};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    io,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        res::{self, ConnectionExt as _},
        xproto::*,
    },
    rust_connection::{DefaultStream, PollMode, RustConnection, Stream},
    utils::RawFdContainer,
};
use x11rb_protocol::{
    parse_display::{ConnectAddress, parse_display},
    xauth::get_auth,
};

struct BoundedStream {
    inner: DefaultStream,
    deadline: Instant,
}
impl BoundedStream {
    fn check(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "X11 window request expired",
            ))
        } else {
            Ok(())
        }
    }
}
impl Stream for BoundedStream {
    fn poll(&self, mode: PollMode) -> io::Result<()> {
        loop {
            self.check()?;
            let mut fd = libc::pollfd {
                fd: self.inner.as_raw_fd(),
                events: (if mode.readable() { libc::POLLIN } else { 0 })
                    | (if mode.writable() { libc::POLLOUT } else { 0 }),
                revents: 0,
            };
            let millis = self
                .deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(100) as i32;
            let n = unsafe { libc::poll(&mut fd, 1, millis) };
            if n > 0 {
                return self.check();
            }
            if n < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
    fn read(&self, buffer: &mut [u8], fds: &mut Vec<RawFdContainer>) -> io::Result<usize> {
        self.check()?;
        self.inner.read(buffer, fds)
    }
    fn write(&self, buffer: &[u8], fds: &mut Vec<RawFdContainer>) -> io::Result<usize> {
        self.check()?;
        self.inner.write(buffer, fds)
    }
}
struct Desktop {
    connection: RustConnection<BoundedStream>,
    root: Window,
}
impl Desktop {
    fn connect() -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let parsed = parse_display(None)?;
        let socket = parsed
            .connect_instruction()
            .find_map(|address| match address {
                ConnectAddress::Socket(path) => Some(path),
                _ => None,
            })
            .context("Window control requires a local X11 socket")?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let unix = runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    tokio::net::UnixStream::connect(socket),
                )
                .await
            })??
            .into_std()?;
        let (stream, (family, address)) = DefaultStream::from_unix_stream(unix)?;
        // Match x11rb's standard connection: a server using local peer access
        // may have no authority file. The server still decides authentication.
        let (name, data) = get_auth(family, &address, parsed.display)
            .unwrap_or(None)
            .unwrap_or_default();
        let connection = RustConnection::connect_to_stream_with_auth_info(
            BoundedStream {
                inner: stream,
                deadline,
            },
            usize::from(parsed.screen),
            name,
            data,
        )?;
        let root = connection.setup().roots[usize::from(parsed.screen)].root;
        let desktop = Self { connection, root };
        let check = desktop.atom(b"_NET_SUPPORTING_WM_CHECK")?;
        let owner = desktop.property(root, check, AtomEnum::WINDOW.into(), 1)?;
        ensure!(
            owner.len() == 1 && owner[0] != 0,
            "An EWMH window manager is required for X11 window control"
        );
        ensure!(
            desktop.property(owner[0], check, AtomEnum::WINDOW.into(), 1)? == owner,
            "X11 window manager identity is stale"
        );
        let version = desktop
            .connection
            .res_query_version(1, 2)?
            .reply()
            .context("X11 window control requires the XRes extension")?;
        ensure!(
            (version.server_major, version.server_minor) >= (1, 2),
            "XRes 1.2 is required to verify local window ownership"
        );
        Ok(desktop)
    }
    fn atom(&self, name: &[u8]) -> Result<Atom> {
        Ok(self.connection.intern_atom(false, name)?.reply()?.atom)
    }
    fn property(&self, window: Window, atom: Atom, kind: Atom, limit: u32) -> Result<Vec<u32>> {
        let value = self
            .connection
            .get_property(false, window, atom, kind, 0, limit)?
            .reply()?;
        if value.type_ == x11rb::NONE {
            return Ok(vec![]);
        }
        ensure!(
            value.type_ == kind && value.format == 32 && value.bytes_after == 0,
            "Malformed or oversized X11 window property"
        );
        Ok(value
            .value32()
            .context("Invalid X11 property values")?
            .collect())
    }
    fn pid(&self, window: Window) -> Result<Option<u32>> {
        let reply = self
            .connection
            .res_query_client_ids(&[res::ClientIdSpec {
                client: window,
                mask: res::ClientIdMask::LOCAL_CLIENT_PID,
            }])?
            .reply()?;
        ensure!(reply.ids.len() <= 1, "Ambiguous X11 client identity");
        Ok(reply
            .ids
            .first()
            .filter(|id| id.spec.mask == res::ClientIdMask::LOCAL_CLIENT_PID && id.value.len() == 1)
            .map(|id| id.value[0])
            .filter(|pid| *pid > 1))
    }
    fn windows(&self, processes: &[PinnedProcess]) -> Result<Vec<Target>> {
        let clients = self.atom(b"_NET_CLIENT_LIST")?;
        let supported = self.property(
            self.root,
            self.atom(b"_NET_SUPPORTED")?,
            AtomEnum::ATOM.into(),
            4096,
        )?;
        ensure!(
            supported.contains(&clients),
            "X11 window manager does not provide a client inventory"
        );
        let inventory = self
            .connection
            .get_property(false, self.root, clients, AtomEnum::WINDOW, 0, 4096)?
            .reply()?;
        ensure!(
            inventory.type_ == u32::from(AtomEnum::WINDOW)
                && inventory.format == 32
                && inventory.bytes_after == 0,
            "X11 client inventory is missing, malformed or oversized"
        );
        let windows: Vec<_> = inventory
            .value32()
            .context("Invalid X11 client inventory")?
            .collect();
        let type_atom = self.atom(b"_NET_WM_WINDOW_TYPE")?;
        let normal = self.atom(b"_NET_WM_WINDOW_TYPE_NORMAL")?;
        let dialog = self.atom(b"_NET_WM_WINDOW_TYPE_DIALOG")?;
        let protocols = self.atom(b"WM_PROTOCOLS")?;
        let delete = self.atom(b"WM_DELETE_WINDOW")?;
        let pids: HashSet<_> = processes.iter().map(|p| p.pid).collect();
        let mut seen = HashSet::new();
        let mut targets = vec![];
        for window in windows {
            if !seen.insert(window) {
                continue;
            }
            // A disappearing window aborts this inventory rather than falsely
            // turning a GUI process into a headless SIGTERM fallback.
            let Some(pid) = self.pid(window)? else {
                continue;
            };
            if !pids.contains(&pid) {
                continue;
            }
            let types = self.property(window, type_atom, AtomEnum::ATOM.into(), 32)?;
            let attributes = self.connection.get_window_attributes(window)?.reply()?;
            // Types are ordered preferences, not independent classifications.
            // Qt utility windows can advertise NORMAL as a fallback.
            let kind = types
                .iter()
                .find(|kind| supported.contains(kind))
                .copied()
                .unwrap_or(normal);
            let eligible = !attributes.override_redirect && (kind == normal || kind == dialog);
            let graceful = self
                .property(window, protocols, AtomEnum::ATOM.into(), 256)?
                .contains(&delete);
            targets.push(Target {
                window,
                pid,
                eligible,
                graceful,
            });
        }
        Ok(targets)
    }
    fn still_owned(&self, target: &Target, processes: &[PinnedProcess]) -> Result<bool> {
        Ok(processes
            .iter()
            .any(|p| p.pid == target.pid && p.matches_now())
            && self.pid(target.window)? == Some(target.pid))
    }
}
struct Target {
    window: Window,
    pid: u32,
    eligible: bool,
    graceful: bool,
}
pub fn close(processes: &[PinnedProcess]) -> Result<CloseResult> {
    let desktop = Desktop::connect()?;
    let targets = desktop.windows(processes)?;
    let found = targets.len();
    let mut requested = 0;
    let protocols = desktop.atom(b"WM_PROTOCOLS")?;
    let delete = desktop.atom(b"WM_DELETE_WINDOW")?;
    for target in targets {
        if target.eligible && target.graceful && desktop.still_owned(&target, processes)? {
            // Deliver only the advertised graceful protocol. Never ask a WM to
            // kill clients that lack WM_DELETE_WINDOW.
            let event = ClientMessageEvent::new(
                32,
                target.window,
                protocols,
                [delete, x11rb::CURRENT_TIME, 0, 0, 0],
            );
            desktop
                .connection
                .send_event(false, target.window, EventMask::NO_EVENT, event)?
                .check()?;
            desktop.connection.flush()?;
            requested += 1;
        }
    }
    Ok(CloseResult { found, requested })
}
pub fn activate(processes: &[PinnedProcess]) -> Result<u32> {
    let desktop = Desktop::connect()?;
    let targets = desktop.windows(processes)?;
    let active = desktop.atom(b"_NET_ACTIVE_WINDOW")?;
    let supported = desktop.property(
        desktop.root,
        desktop.atom(b"_NET_SUPPORTED")?,
        AtomEnum::ATOM.into(),
        4096,
    )?;
    ensure!(
        supported.contains(&active),
        "X11 window manager does not support activation requests"
    );
    for target in targets {
        if target.eligible && desktop.still_owned(&target, processes)? {
            let window_desktop = desktop.atom(b"_NET_WM_DESKTOP")?;
            let current_desktop = desktop.atom(b"_NET_CURRENT_DESKTOP")?;
            let desktop_count = desktop.atom(b"_NET_NUMBER_OF_DESKTOPS")?;
            if [window_desktop, current_desktop, desktop_count]
                .iter()
                .all(|atom| supported.contains(atom))
            {
                let destination = desktop.property(
                    target.window,
                    window_desktop,
                    AtomEnum::CARDINAL.into(),
                    1,
                )?;
                let current = desktop.property(
                    desktop.root,
                    current_desktop,
                    AtomEnum::CARDINAL.into(),
                    1,
                )?;
                let count =
                    desktop.property(desktop.root, desktop_count, AtomEnum::CARDINAL.into(), 1)?;
                if let ([destination], [current], [count]) =
                    (destination.as_slice(), current.as_slice(), count.as_slice())
                    && destination != current
                    && destination < count
                    && desktop.still_owned(&target, processes)?
                {
                    // Some WMs activate only on the current workspace. Sticky
                    // windows (0xffffffff) never request a workspace change.
                    let event = ClientMessageEvent::new(
                        32,
                        desktop.root,
                        current_desktop,
                        [*destination, x11rb::CURRENT_TIME, 0, 0, 0],
                    );
                    desktop
                        .connection
                        .send_event(
                            false,
                            desktop.root,
                            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                            event,
                        )?
                        .check()?;
                }
            }
            if !desktop.still_owned(&target, processes)? {
                continue;
            }
            // EWMH source 2 is for tools representing direct user actions.
            let event = ClientMessageEvent::new(
                32,
                target.window,
                active,
                [2, x11rb::CURRENT_TIME, 0, 0, 0],
            );
            desktop
                .connection
                .send_event(
                    false,
                    desktop.root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    event,
                )?
                .check()?;
            desktop.connection.flush()?;
            return Ok(1);
        }
    }
    Ok(0)
}
