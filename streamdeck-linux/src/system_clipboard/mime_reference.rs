//! Isolated backend reference: raw MIME restoration and per-call reader races.
//! This does not enable restoration in the product.
use super::*;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use wl_clipboard_rs::{
    copy::{self, MimeSource, MimeType as CopyMime, Options, Source},
    paste::{self, ClipboardType, MimeType, Seat},
};

type Bundle = BTreeMap<String, Vec<u8>>;
fn private_root() -> Result<PathBuf> {
    let root = PathBuf::from(
        std::env::var_os("DECKFORGE_PRIVATE_MIME_TEST")
            .context("Run tests/clipboard_mime_reference.py")?,
    );
    ensure!(
        root.is_absolute()
            && root.starts_with("/tmp")
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("deckforge-mime-"),
        "Private fixture root required"
    );
    ensure!(
        fs::metadata(&root)?.uid() == unsafe { libc::getuid() },
        "Private fixture owner mismatch"
    );
    ensure!(
        std::env::var_os("XDG_RUNTIME_DIR") == Some(root.join("runtime").into_os_string())
            && std::env::var_os("DISPLAY").is_none()
            && std::env::var("WAYLAND_DISPLAY")? == "deckforge-private-mime",
        "Private display required"
    );
    let peer =
        std::os::unix::net::UnixStream::connect(root.join("runtime/deckforge-private-mime"))?;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            peer.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    let expected: i32 = std::env::var("DECKFORGE_PRIVATE_KWIN_PID")?.parse()?;
    ensure!(
        status == 0
            && expected > 1
            && credentials.pid == expected
            && credentials.uid == unsafe { libc::getuid() },
        "Compositor is not the owned fixture process"
    );
    Ok(root)
}
fn read(mime: &str) -> Result<Vec<u8>> {
    let (pipe, actual) = paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(mime),
    )?;
    ensure!(actual == mime, "Reader changed MIME type");
    let mut bytes = Vec::new();
    pipe.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "Fixture format exceeded bound"
    );
    Ok(bytes)
}
fn snapshot() -> Result<Bundle> {
    let formats = paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified)?;
    ensure!(formats.len() <= 32, "Fixture format count exceeded bound");
    formats
        .into_iter()
        .map(|mime| Ok((mime.clone(), read(&mime)?)))
        .collect()
}
fn serve(bundle: &Bundle) -> Result<thread::JoinHandle<Result<()>>> {
    serve_on(bundle, copy::ClipboardType::Regular)
}
fn serve_on(
    bundle: &Bundle,
    clipboard: copy::ClipboardType,
) -> Result<thread::JoinHandle<Result<()>>> {
    let mut options = Options::new();
    options
        .foreground(true)
        .omit_additional_text_mime_types(true);
    let read_clipboard = if matches!(clipboard, copy::ClipboardType::Primary) {
        ClipboardType::Primary
    } else {
        ClipboardType::Regular
    };
    options.clipboard(clipboard);
    let prepared = options.prepare_copy_multi(
        bundle
            .iter()
            .map(|(mime, bytes)| MimeSource {
                mime_type: CopyMime::Specific(mime.clone()),
                source: Source::Bytes(bytes.clone().into_boxed_slice()),
            })
            .collect(),
    )?;
    let worker = thread::spawn(move || Ok(prepared.serve()?));
    let deadline = Instant::now() + Duration::from_secs(2);
    let (first, expected) = bundle
        .first_key_value()
        .context("Nonempty fixture source required")?;
    loop {
        if paste::get_mime_types(read_clipboard, Seat::Unspecified)
            .is_ok_and(|types| types == bundle.keys().cloned().collect())
        {
            let (pipe, _) =
                paste::get_contents(read_clipboard, Seat::Unspecified, MimeType::Specific(first))?;
            let mut bytes = Vec::new();
            pipe.take(1024 * 1024).read_to_end(&mut bytes)?;
            if &bytes == expected {
                break;
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Fixture clipboard source did not become ready"
        );
        thread::sleep(Duration::from_millis(5));
    }
    Ok(worker)
}
fn joined(handle: thread::JoinHandle<Result<()>>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !handle.is_finished() {
        ensure!(
            Instant::now() < deadline,
            "Previous clipboard owner did not observe takeover"
        );
        thread::sleep(Duration::from_millis(5));
    }
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("Clipboard fixture thread panicked"))?
}
#[test]
#[ignore = "Requires tests/clipboard_mime_reference.py and its owned Wayland compositor"]
fn private_wayland_multi_mime_reference() -> Result<()> {
    let root = private_root()?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        3,
        2,
        image::Rgba([42, 99, 150, 128]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)?;
    let original: Bundle = [
        (
            "text/plain;charset=utf-8".into(),
            "Exact 🌍\ntext\t".as_bytes().to_vec(),
        ),
        (
            "text/html".into(),
            "<p><b>Exact 🌍</b></p>".as_bytes().to_vec(),
        ),
        (
            "text/uri-list".into(),
            b"file:///tmp/owned%20fixture.txt\r\n".to_vec(),
        ),
        ("image/png".into(), png.into_inner()),
        (
            "application/x-deckforge-fixture".into(),
            vec![0, 1, 2, 127, 128, 255],
        ),
    ]
    .into_iter()
    .collect();
    let owner = serve(&original)?;
    let mut coherent = super::mime_wayland::Clipboard::connect(std::sync::Arc::new(
        std::sync::atomic::AtomicBool::new(false),
    ))?;
    let saved = coherent.capture()?;
    ensure!(saved == original, "Initial MIME bundle changed");
    let wlr_protocol = if coherent.supports_wlr() {
        let mut wlr = super::mime_wayland::Clipboard::connect_protocol(
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            false,
        )?;
        ensure!(
            wlr.capture()? == original,
            "Wlr capture changed raw MIME bytes"
        );
        Some(wlr.protocol())
    } else {
        None
    };
    let many: Bundle = (0..33)
        .map(|i| (format!("application/x-deckforge-bound-{i}"), vec![i]))
        .collect();
    let primary = serve_on(&many, copy::ClipboardType::Primary)?;
    ensure!(
        coherent.capture()? == original,
        "Unrelated primary format limit broke regular capture"
    );
    copy::clear(copy::ClipboardType::Primary, copy::Seat::All)?;
    joined(primary)?;
    let mut clipboard = None;
    set_text(&mut clipboard, "Temporary Text action".into())?;
    joined(owner)?;
    ensure!(
        read("text/plain;charset=utf-8")? == b"Temporary Text action",
        "Product text write did not take over"
    );
    let restored = serve(&saved)?;
    ensure!(snapshot()? == original, "Restoration lost raw MIME bytes");
    set_text(&mut clipboard, "New user copy".into())?;
    joined(restored)?;
    ensure!(
        read("text/plain;charset=utf-8")? == b"New user copy",
        "External takeover was not preserved"
    );
    // Separate get_contents calls bind separate clipboard offers. Identical MIME
    // lists cannot prove one coherent snapshot when ownership changes mid-read.
    let first: Bundle = [
        ("text/plain".into(), b"Old text".to_vec()),
        ("text/html".into(), b"<b>Old HTML</b>".to_vec()),
    ]
    .into_iter()
    .collect();
    let old_owner = serve(&first)?;
    let old_types = paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified)?;
    let text = read("text/plain")?;
    let newer: Bundle = [
        ("text/plain".into(), b"New text".to_vec()),
        ("text/html".into(), b"<i>New HTML</i>".to_vec()),
    ]
    .into_iter()
    .collect();
    let new_owner = serve(&newer)?;
    joined(old_owner)?;
    let html = read("text/html")?;
    ensure!(
        old_types == paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified)?,
        "Fixture format lists should be identical"
    );
    ensure!(
        text == first["text/plain"] && html == newer["text/html"],
        "Expected independent-read generation change"
    );
    let mut changed_owner = None;
    let rejected = coherent.capture_with(|| {
        if changed_owner.is_none() {
            changed_owner = Some(serve(&first).unwrap());
        }
    });
    ensure!(
        rejected
            .unwrap_err()
            .to_string()
            .contains("changed while capturing"),
        "Offer-scoped capture accepted mixed generations"
    );
    joined(new_owner)?;
    ensure!(
        coherent.capture()? == first,
        "Capture did not recover after owner change"
    );
    copy::clear(copy::ClipboardType::Regular, copy::Seat::All)?;
    joined(changed_owner.unwrap())?;
    ensure!(
        coherent.capture()?.is_empty(),
        "Empty clipboard was not distinguished"
    );
    let excessive = serve(&many)?;
    ensure!(
        coherent
            .capture()
            .unwrap_err()
            .to_string()
            .contains("format list exceeds"),
        "Excess formats were accepted"
    );
    copy::clear(copy::ClipboardType::Regular, copy::Seat::All)?;
    joined(excessive)?;
    // Pause only this test's child after it has proved clipboard ownership.
    struct OwnedSource(std::process::Child);
    impl Drop for OwnedSource {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let log = fs::File::create(root.join("source.log"))?;
    let stalled = OwnedSource(
        std::process::Command::new(std::env::current_exe()?)
            .args([
                "system_clipboard::mime_reference::private_wayland_stalling_owner",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env("DECKFORGE_MIME_STALL_CHILD", "1")
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while read("application/x-stalled").ok() != Some(vec![1, 2, 3]) {
        ensure!(Instant::now() < deadline, "Owned stalling source not ready");
        thread::sleep(Duration::from_millis(5));
    }
    ensure!(
        unsafe { libc::kill(stalled.0.id() as i32, libc::SIGSTOP) } == 0,
        "Stop owned clipboard source"
    );
    let mut status = 0;
    ensure!(
        unsafe { libc::waitpid(stalled.0.id() as i32, &mut status, libc::WUNTRACED) }
            == stalled.0.id() as i32
            && libc::WIFSTOPPED(status),
        "Owned source did not stop"
    );
    let began = Instant::now();
    ensure!(
        coherent
            .capture()
            .unwrap_err()
            .to_string()
            .contains("timed out"),
        "Stalled source did not time out"
    );
    ensure!(
        began.elapsed() < Duration::from_secs(4),
        "Receive deadline was not bounded"
    );
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut cancellable = super::mime_wayland::Clipboard::connect(cancelled.clone())?;
    let signal = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        cancelled.store(true, std::sync::atomic::Ordering::Release);
    });
    let began = Instant::now();
    ensure!(
        cancellable
            .capture()
            .unwrap_err()
            .to_string()
            .contains("cancelled"),
        "Cancellation was ignored"
    );
    signal.join().unwrap();
    ensure!(
        began.elapsed() < Duration::from_millis(500),
        "Cancellation waited for receive timeout"
    );
    drop(stalled);
    let healthy = serve(&original)?;
    ensure!(
        coherent.capture()? == original,
        "Connection did not recover after timeout"
    );
    copy::clear(copy::ClipboardType::Regular, copy::Seat::All)?;
    joined(healthy)?;
    fs::write(
        root.join("proof.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "format_count_bounded":true,"primary_isolated":true,"wlr_capture":wlr_protocol.is_some(),"preferred_protocol":coherent.protocol(),"wlr_protocol":wlr_protocol,"stalled_source_bounded":true,"cancelled_capture_bounded":true,"capture_recovers_after_timeout":true,"empty_clipboard":true,"offer_scoped_capture_rejects_changes":true,"capture_recovers_after_change":true,"raw_mime_round_trip":true,"formats":["text/plain;charset=utf-8","text/html","text/uri-list","image/png","application/x-deckforge-fixture"],
            "product_text_writer_exercised":true,"production_restore_enabled":false,"owner_takeover_observed":true,
            "per_call_reader_mixes_generations":true,"identical_mime_lists_do_not_detect_changes":true,"user_clipboard_used":false,"usb_access":false
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "Owned child of the private MIME reference fixture"]
fn private_wayland_stalling_owner() -> Result<()> {
    private_root()?;
    ensure!(
        std::env::var("DECKFORGE_MIME_STALL_CHILD").as_deref() == Ok("1"),
        "Run through the private reference parent"
    );
    let mut options = Options::new();
    options
        .foreground(true)
        .omit_additional_text_mime_types(true);
    options
        .prepare_copy_multi(vec![MimeSource {
            mime_type: CopyMime::Specific("application/x-stalled".into()),
            source: Source::Bytes(vec![1, 2, 3].into_boxed_slice()),
        }])?
        .serve()?;
    Ok(())
}
