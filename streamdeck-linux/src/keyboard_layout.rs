//! Read-only Wayland/KDE or X11 layout translation. Never captures or sends input.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{CString, c_char, c_void},
    fs::File,
    os::fd::{AsFd, AsRawFd},
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_keyboard, wl_registry, wl_seat},
};

#[derive(Default)]
struct Snapshot {
    seat: Option<(u32, wl_seat::WlSeat)>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    map: Option<Vec<u8>>,
    error: Option<String>,
}
impl Dispatch<wl_registry::WlRegistry, ()> for Snapshot {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" => {
                if s.seat.is_some() {
                    s.error = Some("Logical keys require one unambiguous Wayland seat".into());
                } else {
                    s.seat = Some((name, r.bind(name, version.min(4), q, ())));
                }
            }
            wl_registry::Event::GlobalRemove { name }
                if s.seat.as_ref().is_some_and(|(id, _)| *id == name) =>
            {
                s.error = Some("Keyboard seat disappeared".into())
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for Snapshot {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Keyboard) && s.keyboard.is_none() {
                s.keyboard = Some(seat.get_keyboard(q, ()));
            } else if !caps.contains(wl_seat::Capability::Keyboard) {
                s.error = Some("Wayland seat has no keyboard".into());
            }
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Snapshot {
    fn event(
        s: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                let result = (|| -> Result<Vec<u8>> {
                    ensure!(
                        (1..=1024 * 1024).contains(&size),
                        "Desktop keymap exceeds 1 MiB"
                    );
                    let file = File::from(fd);
                    ensure!(
                        file.metadata()?.len() >= u64::from(size),
                        "Truncated desktop keymap"
                    );
                    // The borrowed file offset is never changed. Wayland specifies private mapping.
                    let map = unsafe {
                        libc::mmap(
                            std::ptr::null_mut(),
                            size as usize,
                            libc::PROT_READ,
                            libc::MAP_PRIVATE,
                            file.as_raw_fd(),
                            0,
                        )
                    };
                    ensure!(map != libc::MAP_FAILED, "Map desktop keyboard layout");
                    let bytes = unsafe {
                        std::slice::from_raw_parts(map.cast::<u8>(), size as usize).to_vec()
                    };
                    unsafe {
                        libc::munmap(map, size as usize);
                    }
                    Ok(bytes)
                })();
                match result {
                    Ok(map) => s.map = Some(map),
                    Err(e) => s.error = Some(e.to_string()),
                }
            }
            wl_keyboard::Event::Keymap { .. } => {
                s.error = Some("Unsupported desktop keymap format".into())
            }
            wl_keyboard::Event::Enter { .. } | wl_keyboard::Event::Key { .. } => {
                s.error = Some("Unexpected input on read-only keymap connection".into())
            }
            _ => {}
        }
    }
}
fn read_map(connection: Connection, deadline: Instant) -> Result<Vec<u8>> {
    let mut queue = connection.new_event_queue();
    let q = queue.handle();
    let _registry = connection.display().get_registry(&q, ());
    let mut state = Snapshot::default();
    loop {
        queue.dispatch_pending(&mut state)?;
        if let Some(error) = state.error.take() {
            bail!(error);
        }
        if let Some(map) = state.map.take() {
            return Ok(map);
        }
        ensure!(Instant::now() < deadline, "Desktop keyboard map timed out");
        connection.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd {
                fd: connection.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let millis = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(100) as i32;
            let ready = unsafe { libc::poll(&mut fd, 1, millis) };
            if ready > 0 {
                guard.read()?;
            } else if ready < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: u32) -> *mut c_void;
    fn xkb_context_unref(context: *mut c_void);
    fn xkb_keymap_new_from_string(
        context: *mut c_void,
        text: *const c_char,
        format: u32,
        flags: u32,
    ) -> *mut c_void;
    fn xkb_keymap_unref(map: *mut c_void);
    fn xkb_keymap_num_layouts(map: *mut c_void) -> u32;
    fn xkb_keymap_num_levels_for_key(map: *mut c_void, key: u32, layout: u32) -> u32;
    fn xkb_keymap_mod_get_index(map: *mut c_void, name: *const c_char) -> u32;
    fn xkb_keymap_key_get_syms_by_level(
        map: *mut c_void,
        key: u32,
        layout: u32,
        level: u32,
        out: *mut *const u32,
    ) -> i32;
    fn xkb_keymap_key_get_mods_for_level(
        map: *mut c_void,
        key: u32,
        layout: u32,
        level: u32,
        masks: *mut u32,
        size: usize,
    ) -> usize;
}
struct Keymap(*mut c_void);
impl Drop for Keymap {
    fn drop(&mut self) {
        unsafe {
            xkb_keymap_unref(self.0);
        }
    }
}
impl Keymap {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let text = CString::new(bytes.strip_suffix(&[0]).unwrap_or(bytes))
            .context("Invalid desktop keymap text")?;
        // Each context/keymap belongs to this call and never crosses a thread boundary.
        let context = unsafe { xkb_context_new(0) };
        ensure!(!context.is_null(), "Create keyboard layout context");
        let map = unsafe { xkb_keymap_new_from_string(context, text.as_ptr(), 1, 0) };
        unsafe {
            xkb_context_unref(context);
        }
        ensure!(!map.is_null(), "Parse desktop keyboard map");
        Ok(Self(map))
    }
    fn symbol(&self, key: u16, group: u32, level: u32) -> Option<u32> {
        let mut symbols = std::ptr::null();
        let count = unsafe {
            xkb_keymap_key_get_syms_by_level(self.0, u32::from(key) + 8, group, level, &mut symbols)
        };
        if count == 1 && !symbols.is_null() {
            Some(unsafe { *symbols })
        } else {
            None
        }
    }
    fn mask(&self, name: &std::ffi::CStr) -> u32 {
        let index = unsafe { xkb_keymap_mod_get_index(self.0, name.as_ptr()) };
        1u32.checked_shl(index).unwrap_or(0)
    }
    fn resolve(&self, keys: &[u16], group: u32) -> Result<Vec<u16>> {
        ensure!(
            group < unsafe { xkb_keymap_num_layouts(self.0) },
            "Active layout is outside the received keymap"
        );
        let shift = self.mask(c"Shift");
        let altgr = self.mask(c"Mod5");
        let can_shift = self.symbol(42, group, 0) == Some(0xffe1);
        let can_altgr = self.symbol(100, group, 0) == Some(0xfe03);
        let allowed = if can_shift { shift } else { 0 } | if can_altgr { altgr } else { 0 };
        let mut result = Vec::new();
        let mut extra = 0;
        for &original in keys {
            let Some(symbol) = crate::system_keyboard::printable_symbol(original) else {
                result.push(original);
                continue;
            };
            let mut best = None;
            for key in 1..=255u16 {
                if !crate::system_keyboard::supported(key) {
                    continue;
                }
                let levels =
                    unsafe { xkb_keymap_num_levels_for_key(self.0, u32::from(key) + 8, group) }
                        .min(8);
                for level in 0..levels {
                    if self.symbol(key, group, level) != Some(symbol) {
                        continue;
                    }
                    let mut masks = [0u32; 16];
                    let count = unsafe {
                        xkb_keymap_key_get_mods_for_level(
                            self.0,
                            u32::from(key) + 8,
                            group,
                            level,
                            masks.as_mut_ptr(),
                            masks.len(),
                        )
                    };
                    for &mask in &masks[..count.min(masks.len())] {
                        if mask & !allowed != 0 {
                            continue;
                        }
                        let cost = mask.count_ones() * 1024
                            + u32::from(key != original) * 256
                            + u32::from(key);
                        if best.is_none_or(|(old, _, _)| cost < old) {
                            best = Some((cost, key, mask));
                        }
                    }
                }
            }
            let (_,key,mask)=best.with_context(||format!("Key {} is unavailable in the active layout; choose physical mapping or another key",char::from_u32(symbol).unwrap_or('?')))?;
            result.push(key);
            extra |= mask;
        }
        if extra & shift != 0 && !result.iter().any(|k| matches!(k, 42 | 54)) {
            result.insert(0, 42);
        }
        if extra & altgr != 0 && !result.contains(&100) {
            result.insert(0, 100);
        }
        Ok(result)
    }
}

struct Request {
    keys: Vec<u16>,
    deadline: Instant,
    reply: mpsc::SyncSender<Result<Vec<u16>>>,
}

/// One bounded metadata-only worker isolates potentially blocking display libraries.
/// Late results are discarded; this worker never acquires or emits keyboard input.
pub fn resolve(keys: &[u16], cancelled: &AtomicBool) -> Result<Vec<u16>> {
    static LOOKUPS: OnceLock<std::result::Result<mpsc::SyncSender<Request>, String>> =
        OnceLock::new();
    let sender = LOOKUPS
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(1);
            std::thread::Builder::new()
                .name("keyboard-layout".into())
                .spawn(move || {
                    while let Ok(request) = receiver.recv() {
                        let result = if Instant::now() > request.deadline {
                            Err(anyhow::anyhow!("Keyboard layout request expired"))
                        } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                            resolve_wayland(&request.keys)
                        } else {
                            resolve_x11(&request.keys)
                        };
                        let _ = request.reply.send(result);
                    }
                })
                .map(|_| sender)
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("Start keyboard layout reader: {e}"))?;
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "Keyboard action cancelled"
    );
    let deadline = Instant::now() + Duration::from_millis(500);
    let (reply, result) = mpsc::sync_channel(1);
    sender
        .try_send(Request {
            keys: keys.to_vec(),
            deadline,
            reply,
        })
        .context("Keyboard layout reader is busy or unavailable")?;
    loop {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "Keyboard action cancelled"
        );
        ensure!(
            Instant::now() < deadline,
            "Keyboard layout lookup timed out"
        );
        match result.recv_timeout(Duration::from_millis(5)) {
            Ok(result) => {
                ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "Keyboard action cancelled"
                );
                return result;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("Keyboard layout reader stopped"),
        }
    }
}

fn resolve_wayland(keys: &[u16]) -> Result<Vec<u16>> {
    let display = std::env::var_os("WAYLAND_DISPLAY")
        .context("Desktop layout mapping requires Wayland; choose physical keys on this desktop")?;
    let path = PathBuf::from(display);
    let path = if path.is_absolute() {
        path
    } else {
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("Wayland runtime unavailable")?)
            .join(path)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let stream = runtime
        .block_on(async {
            tokio::time::timeout(
                Duration::from_millis(150),
                tokio::net::UnixStream::connect(path),
            )
            .await
        })??
        .into_std()?;
    let map = read_map(
        Connection::from_socket(stream)?,
        Instant::now() + Duration::from_millis(150),
    )?;
    let group: u32 = runtime.block_on(async {
        tokio::time::timeout(Duration::from_millis(150), async {
            let connection = zbus::Connection::session().await?;
            let proxy = zbus::Proxy::new(
                &connection,
                "org.kde.keyboard",
                "/Layouts",
                "org.kde.KeyboardLayouts",
            )
            .await?;
            proxy.call("getLayout", &()).await
        })
        .await
        .context("Active keyboard layout query timed out")?
        .context("Desktop layout mapping currently requires KDE Plasma")
    })?;
    Keymap::parse(&map)?.resolve(keys, group)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(C)]
    struct Names {
        rules: *const c_char,
        model: *const c_char,
        layout: *const c_char,
        variant: *const c_char,
        options: *const c_char,
    }
    unsafe extern "C" {
        fn xkb_keymap_new_from_names(
            context: *mut c_void,
            names: *const Names,
            flags: u32,
        ) -> *mut c_void;
    }
    fn named(layout: &str, variant: &str) -> Keymap {
        let layout = CString::new(layout).unwrap();
        let variant = CString::new(variant).unwrap();
        let names = Names {
            rules: std::ptr::null(),
            model: c"pc105".as_ptr(),
            layout: layout.as_ptr(),
            variant: variant.as_ptr(),
            options: c"".as_ptr(),
        };
        let context = unsafe { xkb_context_new(0) };
        assert!(!context.is_null());
        let map = unsafe { xkb_keymap_new_from_names(context, &names, 0) };
        unsafe {
            xkb_context_unref(context);
        }
        assert!(!map.is_null());
        Keymap(map)
    }
    #[test]
    fn logical_characters_preserve_modifiers_and_add_required_levels() {
        assert_eq!(
            named("de", "").resolve(&[97, 54, 21], 0).unwrap(),
            vec![97, 54, 44]
        );
        assert_eq!(
            named("fr", "").resolve(&[29, 2], 0).unwrap(),
            vec![42, 29, 2]
        );
        assert_eq!(
            named("us", "dvorak").resolve(&[29, 47], 0).unwrap(),
            vec![29, 52]
        );
        assert_eq!(named("de", "").resolve(&[97, 71], 0).unwrap(), vec![97, 71]);
        let map = named("us,de", ",");
        assert_eq!(map.resolve(&[21], 0).unwrap(), vec![21]);
        assert_eq!(map.resolve(&[21], 1).unwrap(), vec![44]);
        assert!(map.resolve(&[21], 2).is_err());
    }
    #[test]
    fn malformed_maps_and_unavailable_symbols_are_errors() {
        assert!(Keymap::parse(b"not an xkb map\0").is_err());
        assert!(Keymap::parse(b"bad\0interior\0").is_err());
        // Russian-only maps do not silently substitute a physical Latin key.
        assert!(named("ru", "").resolve(&[30], 0).is_err());
    }
}

#[link(name = "xcb")]
unsafe extern "C" {
    fn xcb_connect(display: *const c_char, screen: *mut i32) -> *mut c_void;
    fn xcb_disconnect(connection: *mut c_void);
    fn xcb_connection_has_error(connection: *mut c_void) -> i32;
}
unsafe extern "C" {
    fn xkb_state_serialize_layout(state: *mut c_void, components: u32) -> u32;
    fn xkb_state_unref(state: *mut c_void);
}
struct XConnection(*mut c_void);
impl Drop for XConnection {
    fn drop(&mut self) {
        unsafe {
            xcb_disconnect(self.0);
        }
    }
}
fn resolve_x11(keys: &[u16]) -> Result<Vec<u16>> {
    let display = CString::new(
        std::env::var("DISPLAY")
            .context("Desktop layout mapping requires an X11 or KDE Wayland display")?,
    )?;
    let api = xkbcommon_dl::x11::xkbcommon_x11_option()
        .context("X11 layout mapping requires libxkbcommon-x11")?;
    // XCB performs the authenticated connection and XKB queries. These blocking
    // calls run only on the bounded metadata worker, never on the input worker.
    let connection = unsafe { xcb_connect(display.as_ptr(), std::ptr::null_mut()) };
    ensure!(!connection.is_null(), "Connect to X11 display");
    let connection = XConnection(connection);
    ensure!(
        unsafe { xcb_connection_has_error(connection.0) } == 0,
        "X11 display connection failed"
    );
    let ok = unsafe {
        (api.xkb_x11_setup_xkb_extension)(connection.0,1,0,xkbcommon_dl::x11::xkb_x11_setup_xkb_extension_flags::XKB_X11_SETUP_XKB_EXTENSION_NO_FLAGS,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut())
    };
    ensure!(ok != 0, "X11 server does not provide XKB");
    let device = unsafe { (api.xkb_x11_get_core_keyboard_device_id)(connection.0) };
    ensure!(device >= 0, "X11 core keyboard unavailable");
    let context = unsafe { xkb_context_new(0) };
    ensure!(!context.is_null(), "Create X11 keymap context");
    let map = unsafe {
        (api.xkb_x11_keymap_new_from_device)(
            context.cast(),
            connection.0,
            device,
            xkbcommon_dl::xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
        )
    };
    unsafe {
        xkb_context_unref(context);
    }
    ensure!(!map.is_null(), "Read X11 keyboard map");
    let map = Keymap(map.cast());
    let state = unsafe { (api.xkb_x11_state_new_from_device)(map.0.cast(), connection.0, device) };
    ensure!(!state.is_null(), "Read active X11 keyboard layout");
    let group = unsafe { xkb_state_serialize_layout(state.cast(), 1 << 7) };
    unsafe {
        xkb_state_unref(state.cast());
    }
    map.resolve(keys, group)
}
