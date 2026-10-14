//! Opt-in real clipboard acceptance. No production keyboard backend is changed.
use super::*;
use std::os::unix::fs::MetadataExt;

fn key_mode() -> &'static str {
    if std::env::var("DECKFORGE_TEST_LOGICAL_KEYS").as_deref() == Ok("1") {
        "logical"
    } else {
        "physical"
    }
}

fn private_root() -> Result<PathBuf> {
    let root = PathBuf::from(
        std::env::var_os("DECKFORGE_PRIVATE_CLIPBOARD_TEST")
            .context("Run the isolated clipboard test script")?,
    );
    ensure!(
        root.is_absolute()
            && root.starts_with("/tmp")
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("deckforge-focused-"),
        "Invalid private fixture directory"
    );
    ensure!(
        fs::metadata(&root)?.uid() == unsafe { libc::geteuid() },
        "Private fixture must belong to this user"
    );
    ensure!(
        std::env::var_os("XDG_RUNTIME_DIR") == Some(root.join("runtime").into_os_string()),
        "Private runtime required"
    );
    if let Ok(display) = std::env::var("DISPLAY") {
        use std::os::fd::AsRawFd;
        ensure!(
            std::env::var_os("WAYLAND_DISPLAY").is_none(),
            "X11 fixture must not target Wayland"
        );
        let number = display
            .strip_prefix(':')
            .context("Private local X11 display required")?;
        ensure!(
            !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()),
            "Invalid private X11 display"
        );
        let expected: i32 = std::env::var("DECKFORGE_PRIVATE_X11_PID")?.parse()?;
        let socket = std::os::unix::net::UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))?;
        let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut peer as *mut libc::ucred).cast(),
                &mut size,
            )
        };
        ensure!(
            status == 0
                && expected > 1
                && peer.pid == expected
                && peer.uid == unsafe { libc::geteuid() },
            "X11 server is not the owned fixture process"
        );
    } else {
        ensure!(
            std::env::var("WAYLAND_DISPLAY").as_deref() == Ok("deckforge-private-input"),
            "Private Wayland socket required"
        );
    }
    ensure!(
        root.join("bridge-ready").is_file(),
        "Owned compositor bridge is not ready"
    );
    Ok(root)
}

#[test]
#[ignore = "Requires tests/focused_clipboard_interop.py and its private compositor"]
fn focused_clipboard_on_private_wayland() -> Result<()> {
    let root = private_root()?;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("keys.bin"))?;
    let worker = Worker::start(Some(crate::system_keyboard::Keyboard::with_test_writer(
        file,
    )));
    let mut sequence = 0u64;
    let mut client = |operation: &str, text: &str, select: bool| -> Result<Value> {
        sequence += 1;
        let request = json!({"id":sequence,"operation":operation,"text":text,"select":select});
        fs::write(
            root.join("client-command.tmp"),
            serde_json::to_vec(&request)?,
        )?;
        fs::rename(
            root.join("client-command.tmp"),
            root.join("client-command.json"),
        )?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(bytes) = fs::read(root.join("client-state.json"))
                && let Ok(state) = serde_json::from_slice::<Value>(&bytes)
                && state["id"] == sequence
            {
                return Ok(state);
            }
            ensure!(Instant::now() < deadline, "Focused client did not answer");
            thread::sleep(Duration::from_millis(5));
        }
    };
    let submit = |plan: Plan| -> Result<()> {
        worker.submit("private-fixture", plan)?;
        worker
            .reports
            .recv_timeout(Duration::from_secs(3))?
            .result?;
        Ok(())
    };
    let check = |client: &mut dyn FnMut(&str, &str, bool) -> Result<Value>,
                 text: &str,
                 clip: Option<&str>|
     -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let state = client("read", "", false)?;
            if state["active"] == true
                && state["focused"] == true
                && state["text"] == text
                && clip.is_none_or(|s| state["clipboard"] == s)
            {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Clipboard acceptance mismatch: {state}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    };
    check(&mut client, "", None)?;
    client("seed", "Selected 🌍 æøå", true)?;
    submit(plan(
        crate::system_clipboard::UUID,
        &json!({"operation":"copy","linuxKeyMode":key_mode()}),
    )?)?;
    check(&mut client, "Selected 🌍 æøå", Some("Selected 🌍 æøå"))?;
    submit(plan(
        crate::system_clipboard::UUID,
        &json!({"operation":"cut","linuxKeyMode":key_mode()}),
    )?)?;
    check(&mut client, "", Some("Selected 🌍 æøå"))?;
    submit(plan(
        crate::system_clipboard::UUID,
        &json!({"operation":"paste","linuxKeyMode":key_mode()}),
    )?)?;
    check(&mut client, "Selected 🌍 æøå", Some("Selected 🌍 æøå"))?;
    client("seed", "", false)?;
    submit(plan(
        TEXT,
        &json!({"pastedText":"Line one\nUnicode 🌍 æøå","isSendingEnter":true,"isTypingMode":false,"linuxKeyMode":key_mode()}),
    )?)?;
    check(
        &mut client,
        "Line one\nUnicode 🌍 æøå\n",
        Some("Line one\nUnicode 🌍 æøå"),
    )?;
    // Copy text must not send any keyboard packets or alter the focused document.
    let before = fs::metadata(root.join("keys.bin"))?.len();
    submit(plan(
        crate::system_clipboard::UUID,
        &json!({"operation":"copy_text","text":"clipboard only"}),
    )?)?;
    check(
        &mut client,
        "Line one\nUnicode 🌍 æøå\n",
        Some("clipboard only"),
    )?;
    ensure!(
        fs::metadata(root.join("keys.bin"))?.len() == before,
        "Copy text emitted keys"
    );
    // A paste keeps clipboard writes ordered, but must not delay an owned key-up.
    let key_released_since = |offset: usize| -> Result<bool> {
        let bytes = fs::read(root.join("keys.bin"))?;
        Ok(bytes[offset..]
            .chunks_exact(std::mem::size_of::<libc::input_event>())
            .any(|event| {
                let end = event.len();
                u16::from_ne_bytes(event[end - 8..end - 6].try_into().unwrap()) == 1
                    && i32::from_ne_bytes(event[end - 4..end].try_into().unwrap()) == 0
            }))
    };
    let await_release = |offset| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !key_released_since(offset)? {
            ensure!(Instant::now() < deadline, "Paste chord was not released");
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    };
    client("seed", "", false)?;
    worker.submit("held-control", Plan::Press(vec![29].into()))?;
    worker
        .reports
        .recv_timeout(Duration::from_secs(2))?
        .result?;
    let offset = fs::metadata(root.join("keys.bin"))?.len() as usize;
    worker.submit(
        "paced-first",
        plan(
            TEXT,
            &json!({"pastedText":"First 🌍", "isSendingEnter":true,"linuxKeyMode":key_mode()}),
        )?,
    )?;
    await_release(offset)?;
    worker.submit("deferred-key", Plan::Press(vec![29].into()))?;
    worker.submit("deferred-key", Plan::Release)?;
    worker.submit("copy-after-paste", Plan::Clipboard("intermediate".into()))?;
    worker.submit("held-control", Plan::Release)?;
    let released = worker.reports.recv_timeout(Duration::from_secs(2))?;
    ensure!(
        released.context == "held-control",
        "Paste delayed a held key-up: {}",
        released.context
    );
    released.result?;
    worker.submit(
        "paced-second",
        plan(
            TEXT,
            &json!({"pastedText":"Second", "linuxKeyMode":key_mode()}),
        )?,
    )?;
    for expected in [
        "paced-first",
        "deferred-key",
        "deferred-key",
        "copy-after-paste",
        "paced-second",
    ] {
        let report = worker.reports.recv_timeout(Duration::from_secs(2))?;
        ensure!(
            report.context == expected,
            "Clipboard FIFO changed: {}",
            report.context
        );
        report.result?;
    }
    check(&mut client, "First 🌍\nSecond", Some("Second"))?;
    client("seed", "Left", false)?;
    submit(plan(
        TEXT,
        &json!({"pastedText":"", "isSendingEnter":true,"linuxKeyMode":key_mode()}),
    )?)?;
    check(&mut client, "Left\n", Some(""))?;
    // Shutdown during the settling interval must suppress the optional Enter.
    let offset = fs::metadata(root.join("keys.bin"))?.len() as usize;
    worker.submit(
        "cancelled-paste",
        plan(
            TEXT,
            &json!({"pastedText":"Stop", "isSendingEnter":true,"linuxKeyMode":key_mode()}),
        )?,
    )?;
    await_release(offset)?;
    worker.cancel();
    drop(worker);
    let bytes = fs::read(root.join("keys.bin"))?;
    ensure!(
        !bytes[offset..]
            .chunks_exact(std::mem::size_of::<libc::input_event>())
            .any(|event| {
                let end = event.len();
                u16::from_ne_bytes(event[end - 8..end - 6].try_into().unwrap()) == 1
                    && u16::from_ne_bytes(event[end - 6..end - 4].try_into().unwrap()) == 28
                    && i32::from_ne_bytes(event[end - 4..end].try_into().unwrap()) == 1
            }),
        "Shutdown sent a delayed Enter"
    );
    fs::write(
        root.join("rust-proof.json"),
        serde_json::to_vec_pretty(
            &json!({"paste_keyup_priority":true,"paste_clipboard_fifo":true,"empty_text_enter_clears_clipboard":true,"shutdown_suppresses_enter":true,"focused_copy":true,"focused_cut":true,"focused_paste":true,"text_unicode_multiline":true,"optional_enter":true,"copy_only_sends_no_keys":true,"keyboard_backend":if std::env::var_os("DISPLAY").is_some(){"test writer -> isolated Xvfb XTest"}else{"test writer -> isolated KWin EIS"},"uinput_opened":false}),
        )?,
    )?;
    Ok(())
}

#[test]
#[ignore = "Requires tests/focused_clipboard_interop.py and its private Konsole"]
fn terminal_clipboard_on_private_wayland() -> Result<()> {
    let root = private_root()?;
    ensure!(
        root.join("terminal-ready.json").is_file(),
        "Private terminal is not ready"
    );
    let file = fs::OpenOptions::new()
        .append(true)
        .open(root.join("keys.bin"))?;
    let worker = Worker::start(Some(crate::system_keyboard::Keyboard::with_test_writer(
        file,
    )));
    let submit = |plan: Plan| -> Result<()> {
        worker.submit("private-terminal", plan)?;
        worker
            .reports
            .recv_timeout(Duration::from_secs(3))?
            .result?;
        Ok(())
    };
    submit(plan(
        crate::system_clipboard::UUID,
        &json!({"operation":"copy_text","text":"Terminal 🌍 æøå"}),
    )?)?;
    submit(plan(
        crate::system_clipboard::UUID,
        &json!({"operation":"paste","linuxKeyMode":key_mode(),"terminal_shortcuts":true}),
    )?)?;
    // Enter is a separate simulated user gesture, after the clipboard read.
    thread::sleep(Duration::from_millis(150));
    submit(Plan::Chord(vec![28].into()))?;
    submit(plan(
        TEXT,
        &json!({"pastedText":"Custom terminal paste 🌍","isSendingEnter":true,"isTypingMode":false,"Hotkey":{"linuxKeyMode":key_mode(),"QTKeyCode":86,"KeyModifiers":8192,"KeyCtrl":true,"KeyShift":true}}),
    )?)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(bytes) = fs::read(root.join("terminal-result.json"))
            && let Ok(result) = serde_json::from_slice::<Value>(&bytes)
        {
            ensure!(result["tty"] == true, "Fixture has no terminal");
            ensure!(
                result["lines"] == json!(["Terminal 🌍 æøå\n", "Custom terminal paste 🌍\n"]),
                "Terminal paste mismatch: {result}"
            );
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Private terminal did not receive both pasted lines"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(worker);
    fs::write(
        root.join("terminal-proof.json"),
        serde_json::to_vec_pretty(
            &json!({"konsole_paste":true,"text_custom_paste_shortcut":true,"uinput_opened":false}),
        )?,
    )?;
    Ok(())
}

#[test]
#[ignore = "Requires logical_hotkeys_interop.py and its owned compositor"]
fn logical_worker_on_private_wayland() -> Result<()> {
    let root = private_root()?;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("keys.bin"))?;
    let worker = Worker::start(Some(crate::system_keyboard::Keyboard::with_test_writer(
        file,
    )));
    let submit = |context: &str, plan: Plan| -> Result<()> {
        worker.submit(context, plan)?;
        worker
            .reports
            .recv_timeout(Duration::from_secs(3))?
            .result?;
        Ok(())
    };
    let chord = |key: u16| crate::system_keyboard::Chord {
        keys: vec![29, key],
        logical: true,
    };
    submit("first", Plan::Press(chord(21)))?;
    submit("second", Plan::Press(chord(44)))?;
    submit("first", Plan::Release)?;
    fs::write(root.join("worker-held"), b"ready")?;
    let wait = |name: &str| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !root.join(name).exists() {
            ensure!(
                Instant::now() < deadline,
                "Private worker test timed out: {name}"
            );
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    };
    wait("worker-release")?;
    submit("second", Plan::Release)?;
    fs::write(root.join("worker-released"), b"ready")?;
    submit(
        "dial",
        Plan::Dial {
            stroke: crate::system_dial::Stroke {
                keys: crate::system_keyboard::Chord {
                    keys: vec![21],
                    logical: true,
                },
                gesture: 0,
                interval: Duration::from_millis(250),
            },
            count: 2,
            coalesce: true,
        },
    )?;
    wait("worker-finish")?;
    drop(worker);
    Ok(())
}

#[test]
#[ignore = "Requires x11_layout_interop.py with its owned stalled display proxy"]
fn layout_lookup_failure_on_private_x11() -> Result<()> {
    let root = PathBuf::from(std::env::var("DECKFORGE_PRIVATE_CLIPBOARD_TEST")?);
    ensure!(
        root.parent() == Some(std::path::Path::new("/tmp"))
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("deckforge-focused-"),
        "Private root required"
    );
    ensure!(
        std::env::var("XDG_RUNTIME_DIR")? == root.join("runtime").to_string_lossy(),
        "Private runtime required"
    );
    ensure!(
        std::env::var("DISPLAY")?.starts_with("127.0.0.1:")
            && std::env::var_os("WAYLAND_DISPLAY").is_none(),
        "Private X11 proxy required"
    );
    let mode = std::env::var("DECKFORGE_LAYOUT_PROBE_MODE")?;
    ensure!(
        matches!(mode.as_str(), "cancel" | "timeout"),
        "Unknown fixture mode"
    );
    let output = root.join("lookup-keys.bin");
    let worker = Worker::start(Some(crate::system_keyboard::Keyboard::with_test_writer(
        fs::File::create(&output)?,
    )));
    let started = Instant::now();
    worker.submit(
        "lookup",
        Plan::Press(crate::system_keyboard::Chord {
            keys: vec![21],
            logical: true,
        }),
    )?;
    let wait = |name: &str| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !root.join(name).exists() {
            ensure!(Instant::now() < deadline, "Fixture did not reach {name}");
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    };
    wait("proxy-accepted")?;
    let cancelled = Instant::now();
    if mode == "cancel" {
        worker.cancel();
    }
    let error = worker
        .reports
        .recv_timeout(Duration::from_secs(2))?
        .result
        .unwrap_err()
        .to_string();
    if mode == "cancel" {
        ensure!(
            error.contains("cancelled") && cancelled.elapsed() < Duration::from_millis(250),
            "Cancellation was not prompt: {error}"
        );
    } else {
        ensure!(
            error.contains("timed out") && started.elapsed() < Duration::from_secs(1),
            "Lookup timeout was not bounded: {error}"
        );
    }
    drop(worker);
    ensure!(
        fs::metadata(&output)?.len() == 0,
        "Failed lookup emitted input"
    );
    let elapsed = if mode == "cancel" {
        cancelled.elapsed()
    } else {
        started.elapsed()
    };
    fs::write(root.join("lookup-failed"), b"ready")?;
    wait("proxy-unblocked")?;
    let keys = crate::keyboard_layout::resolve(&[21], &AtomicBool::new(false))?;
    ensure!(
        !keys.is_empty() && fs::metadata(&output)?.len() == 0,
        "Late result emitted input or reader failed to recover"
    );
    fs::write(
        root.join("lookup-proof.json"),
        serde_json::to_vec(
            &json!({"mode":mode,"elapsed_ms":elapsed.as_millis(),"no_input":true,"late_result_discarded":true,"reader_recovered":true}),
        )?,
    )?;
    Ok(())
}
