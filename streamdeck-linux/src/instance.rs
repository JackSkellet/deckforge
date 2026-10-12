//! Private local IPC for the native window. `spawn` requires the caller to hold
//! the state's session.lock for the entire Server lifetime. Callbacks must only
//! queue commands or read status; they must not block waiting for the GUI.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs,
    future::Future,
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixListener as StdListener,
    },
    path::{Path, PathBuf},
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::oneshot,
    task::JoinSet,
    time::timeout,
};

const LIMIT: usize = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(1);
type Callback = Arc<dyn Fn(Command) -> Result<Value> + Send + Sync>;
type LinkCallback = Arc<dyn Fn(&str) -> Result<Value> + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Show,
    Hide,
    Close,
    Quit,
    Status,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    state: String,
    command: Command,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkMessage {
    state: String,
    uri: String,
}
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Request {
    Window(Message),
    Link(LinkMessage),
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Reply {
    Ok(Value),
    Error(String),
}

/// Returns None only for a missing state/socket or a refused socket connection.
/// A slow, inaccessible, malformed, or mismatched server is an error. Never unlinks.
pub fn request(state: &Path, command: Command) -> Result<Option<Value>> {
    request_message(state, move |state| {
        Request::Window(Message { state, command })
    })
}

pub fn request_deep_link(state: &Path, uri: &str) -> Result<Option<Value>> {
    crate::deep_link::DeepLink::parse(uri)?;
    let uri = uri.to_owned();
    // Validate serialized size even when no endpoint exists.
    line(&LinkMessage {
        state: "0".repeat(64),
        uri: uri.clone(),
    })?;
    request_message(state, move |state| {
        Request::Link(LinkMessage { state, uri })
    })
}

fn request_message(
    state: &Path,
    message: impl FnOnce(String) -> Request + Send,
) -> Result<Option<Value>> {
    let canonical = match state.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Resolve instance state directory"),
    };
    let endpoint = Endpoint::new(&canonical, false)?;
    if socket(&endpoint.path)?.is_none() {
        return Ok(None);
    }
    // A dedicated short-lived runtime also works when the synchronous caller is
    // already inside another Tokio runtime. The complete socket exchange is bounded.
    blocking(async move {
        timeout(TIMEOUT, async {
            let Some(mut stream) = open(&endpoint.path).await? else {
                return Ok(None);
            };
            let bytes = line(&message(endpoint.identity))?;
            stream
                .write_all(&bytes)
                .await
                .context("Send instance command")?;
            let response: Reply = serde_json::from_slice(&read_line(&mut stream).await?)
                .context("Decode instance response")?;
            match response {
                Reply::Ok(value) => Ok(Some(value)),
                Reply::Error(error) => bail!(error),
            }
        })
        .await
        .context("Instance request timed out")?
    })
}

/// The caller must already own the state directory's session.lock. Refuses live
/// sockets and foreign files. Only a refused, unchanged socket may be replaced.
pub fn spawn(state: &Path, callback: Callback) -> Result<Server> {
    spawn_with_deep_links(
        state,
        callback,
        Arc::new(|_| bail!("This editor does not support plugin deep links")),
    )
}

pub fn spawn_with_deep_links(
    state: &Path,
    callback: Callback,
    link_callback: LinkCallback,
) -> Result<Server> {
    let endpoint = Endpoint::new(
        &state
            .canonicalize()
            .context("Resolve instance state directory")?,
        true,
    )?;
    if let Some(previous) = socket(&endpoint.path)? {
        let path = endpoint.path.clone();
        let live = blocking(async move {
            timeout(TIMEOUT, open(&path))
                .await
                .context("Existing instance socket connect timed out")?
        })?;
        ensure!(live.is_none(), "Instance socket is already listening");
        // open() can also return None if the socket disappeared. No unrelated
        // replacement is removed between the refused connection and bind.
        previous.remove_if_same()?;
    }
    // Publish only after permissions are set. A simultaneous launcher must never
    // observe bind's initial umask-dependent mode at the well-known endpoint.
    let temporary = endpoint
        .path
        .with_file_name(format!(".instance-{}.sock", uuid::Uuid::new_v4()));
    let listener = StdListener::bind(&temporary).context("Bind temporary instance socket")?;
    let mut owned = OwnedSocket::inspect(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    owned.publish(&endpoint.path)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let listener = {
        let _entered = runtime.enter();
        UnixListener::from_std(listener)?
    };
    let (stop, stopped) = oneshot::channel();
    let thread = thread::Builder::new()
        .name("deckforge-instance".into())
        .spawn(move || {
            runtime.block_on(serve(
                listener,
                endpoint.identity,
                callback,
                link_callback,
                stopped,
            ));
        })?;
    Ok(Server {
        stop: Some(stop),
        thread: Some(thread),
        _owned: owned,
    })
}

pub struct Server {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    _owned: OwnedSocket,
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // The socket guard drops after the worker joins, also covering failed startup.
    }
}

fn uid() -> u32 {
    // SAFETY: geteuid has no pointers or preconditions and changes no process state.
    unsafe { libc::geteuid() }
}

struct Endpoint {
    path: PathBuf,
    identity: String,
}
impl Endpoint {
    fn new(canonical: &Path, create: bool) -> Result<Self> {
        ensure!(canonical.is_dir(), "Instance state must be a directory");
        let identity = format!("{:x}", Sha256::digest(canonical.as_os_str().as_bytes()));
        let basename = format!("{identity}.sock");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let root = runtime
            .filter(|root| {
                root.is_absolute()
                    && private_directory(root, false).is_ok_and(|exists| exists)
                    && root
                        .join("deckforge")
                        .join(&basename)
                        .as_os_str()
                        .as_bytes()
                        .len()
                        < 108
            })
            .map(|root| root.join("deckforge"))
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/deckforge-{}", uid())));
        private_directory(&root, create)?;
        let path = root.join(basename);
        ensure!(
            path.as_os_str().as_bytes().len() < 108,
            "Instance socket path exceeds Unix socket limit"
        );
        Ok(Self { path, identity })
    }
}

fn private_directory(path: &Path, create: bool) -> Result<bool> {
    if create {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("Create private instance directory"),
        }
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("Inspect private instance directory"),
    };
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == uid()
            && metadata.mode() & 0o777 == 0o700,
        "Instance directory must be an owned, non-symlink directory with mode 0700"
    );
    Ok(true)
}

struct OwnedSocket(SocketIdentity);
impl OwnedSocket {
    fn inspect(path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.file_type().is_socket() && metadata.uid() == uid(),
            "Instance endpoint is not an owned socket"
        );
        Ok(Self(SocketIdentity {
            path: path.to_owned(),
            device: metadata.dev(),
            inode: metadata.ino(),
        }))
    }
    fn publish(&mut self, destination: &Path) -> Result<()> {
        let source = CString::new(self.0.path.as_os_str().as_bytes())?;
        let target = CString::new(destination.as_os_str().as_bytes())?;
        // SAFETY: Both C strings remain live. RENAME_NOREPLACE atomically moves
        // this socket without overwriting any endpoint created after our probe.
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error())
                .context("Publish instance socket without replacing an existing endpoint");
        }
        self.0.path = destination.to_owned();
        Ok(())
    }
}
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        let _ = self.0.remove_if_same();
    }
}
// Inspection must not itself take ownership and unlink on Drop.
fn socket(path: &Path) -> Result<Option<SocketIdentity>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Inspect instance socket"),
    };
    ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == uid()
            && metadata.mode() & 0o777 == 0o600,
        "Instance endpoint must be an owned, non-symlink socket with mode 0600"
    );
    Ok(Some(SocketIdentity {
        path: path.to_owned(),
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}
struct SocketIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl SocketIdentity {
    fn remove_if_same(&self) -> Result<()> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            metadata.file_type().is_socket()
                && metadata.uid() == uid()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode,
            "Instance endpoint changed; refusing to unlink it"
        );
        fs::remove_file(&self.path).context("Remove owned instance socket")
    }
}

async fn open(path: &Path) -> Result<Option<UnixStream>> {
    match UnixStream::connect(path).await {
        Ok(stream) => {
            ensure!(
                stream.peer_cred()?.uid() == uid(),
                "Instance peer belongs to another user"
            );
            Ok(Some(stream))
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error).context("Connect to instance socket"),
    }
}

fn blocking<T: Send>(future: impl Future<Output = Result<T>> + Send) -> Result<T> {
    thread::scope(|scope| {
        scope
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(future)
            })
            .join()
            .map_err(|_| anyhow::anyhow!("Instance request thread panicked"))?
    })
}

fn line(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() < LIMIT, "Instance message exceeds 16 KiB");
    bytes.push(b'\n');
    Ok(bytes)
}
async fn read_line(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 512];
    loop {
        let count = stream.read(&mut buffer).await?;
        ensure!(count != 0, "Instance peer closed without a JSON line");
        let end = buffer[..count].iter().position(|&byte| byte == b'\n');
        let take = end.unwrap_or(count);
        ensure!(
            bytes.len() + take < LIMIT,
            "Instance message exceeds 16 KiB"
        );
        bytes.extend_from_slice(&buffer[..take]);
        if end.is_some() {
            return Ok(bytes);
        }
    }
}

async fn exchange(
    mut stream: UnixStream,
    identity: &str,
    callback: &Callback,
    link_callback: &LinkCallback,
) -> Result<()> {
    ensure!(
        stream.peer_cred()?.uid() == uid(),
        "Instance peer belongs to another user"
    );
    let result = async {
        let message: Request = serde_json::from_slice(&read_line(&mut stream).await?)
            .context("Invalid instance command")?;
        let state = match &message {
            Request::Window(message) => &message.state,
            Request::Link(message) => &message.state,
        };
        ensure!(state == identity, "Instance state identity mismatch");
        match message {
            Request::Window(message) => callback(message.command),
            Request::Link(message) => {
                crate::deep_link::DeepLink::parse(&message.uri)?;
                link_callback(&message.uri)
            }
        }
    }
    .await;
    let reply = match result {
        Ok(value) => Reply::Ok(value),
        Err(error) => Reply::Error(format!("{error:#}")),
    };
    let bytes = line(&reply).unwrap_or_else(|_| {
        b"{\"status\":\"error\",\"value\":\"Instance response exceeds 16 KiB\"}\n".to_vec()
    });
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn serve(
    listener: UnixListener,
    identity: String,
    callback: Callback,
    link_callback: LinkCallback,
    mut stop: oneshot::Receiver<()>,
) {
    let identity = Arc::new(identity);
    let mut clients = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            Some(_) = clients.join_next(), if !clients.is_empty() => {},
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) if clients.len() < 16 => {
                    let identity = identity.clone();
                    let callback = callback.clone();
                    let link_callback = link_callback.clone();
                    clients.spawn(async move { let _ = timeout(TIMEOUT, exchange(stream, &identity, &callback, &link_callback)).await; });
                },
                Ok(_) => {}, // Bound simultaneous clients even if a peer never sends a line.
                Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
            },
        }
    }
    clients.abort_all();
    while clients.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        os::unix::{fs::symlink, net::UnixStream as StdStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    fn callback() -> Callback {
        Arc::new(|command| Ok(json!({"command": command})))
    }
    fn endpoint(state: &Path) -> Endpoint {
        Endpoint::new(&state.canonicalize().unwrap(), true).unwrap()
    }
    fn stale(path: &Path) {
        let listener = StdListener::bind(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        drop(listener);
    }
    fn raw(path: &Path, bytes: &[u8]) -> Reply {
        let mut stream = StdStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(bytes).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        serde_json::from_slice(&response).unwrap()
    }

    #[test]
    fn deep_links_keep_window_protocol_and_route_private_opaque_messages() {
        let state = tempfile::tempdir().unwrap();
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = received.clone();
        let _server = spawn_with_deep_links(
            state.path(),
            callback(),
            Arc::new(move |uri| {
                capture.lock().unwrap().push(uri.to_owned());
                Ok(json!({"accepted": true}))
            }),
        )
        .unwrap();
        let uri = format!(
            "streamdeck://plugins/message/com.example.test/callback?token={}&streamdeck=HIDDEN#Keep%2F",
            "x".repeat(6000)
        );
        assert_eq!(
            request_deep_link(state.path(), &uri).unwrap(),
            Some(json!({"accepted": true}))
        );
        assert_eq!(*received.lock().unwrap(), vec![uri]);
        assert_eq!(
            request(state.path(), Command::Status).unwrap(),
            Some(json!({"command":"status"}))
        );
        let ep = endpoint(state.path());
        let mismatched = line(&LinkMessage {
            state: "other-state".into(),
            uri: "streamdeck://plugins/message/com.example.test/private".into(),
        })
        .unwrap();
        assert!(matches!(raw(&ep.path, &mismatched), Reply::Error(_)));
        let malformed = line(&LinkMessage {
            state: ep.identity,
            uri: "streamdeck://unsupported?token=secret".into(),
        })
        .unwrap();
        match raw(&ep.path, &malformed) {
            Reply::Error(error) => assert!(!error.contains("secret")),
            _ => panic!("Malformed deep link accepted"),
        }
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn deep_link_errors_are_not_missing_instances_and_size_is_bounded() {
        let state = tempfile::tempdir().unwrap();
        let _server = spawn(state.path(), callback()).unwrap();
        let uri = "streamdeck://plugins/message/com.example.test/private?token=secret";
        let error = request_deep_link(state.path(), uri)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not support plugin deep links"));
        assert!(!error.contains("secret"));
        let oversized = format!(
            "streamdeck://plugins/message/com.example.test/{}",
            "x".repeat(LIMIT)
        );
        assert!(request_deep_link(state.path(), &oversized).is_err());
        assert!(request(state.path(), Command::Status).unwrap().is_some());
        let absent = state.path().join("absent");
        assert!(request_deep_link(&absent, uri).unwrap().is_none());
        assert!(!absent.exists());
    }

    #[test]
    fn slow_deep_link_endpoint_is_an_error_without_cleanup_or_fallback() {
        let state = tempfile::tempdir().unwrap();
        let ep = endpoint(state.path());
        let _listener = StdListener::bind(&ep.path).unwrap();
        fs::set_permissions(&ep.path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            request_deep_link(
                state.path(),
                "streamdeck://plugins/message/com.example.test/private"
            )
            .unwrap_err()
            .to_string()
            .contains("timed out")
        );
        assert!(ep.path.exists());
        fs::remove_file(&ep.path).unwrap();
    }

    #[test]
    fn separate_states_aliases_and_long_paths_route_to_the_right_instance() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let long = second.path().join("long-state-directory-".repeat(10));
        fs::create_dir(&long).unwrap();
        let alias = second.path().join("alias");
        symlink(first.path(), &alias).unwrap();
        let a = spawn(first.path(), callback()).unwrap();
        let b = spawn(&long, Arc::new(|_| Ok(json!({"instance": "B"})))).unwrap();
        for command in [
            Command::Show,
            Command::Hide,
            Command::Close,
            Command::Quit,
            Command::Status,
        ] {
            assert_eq!(
                request(&alias, command).unwrap(),
                Some(json!({"command": command}))
            );
        }
        assert_eq!(
            request(&long, Command::Status).unwrap(),
            Some(json!({"instance": "B"}))
        );
        let ea = endpoint(first.path());
        let eb = endpoint(&long);
        assert_ne!(ea.path, eb.path);
        assert!(eb.path.as_os_str().as_bytes().len() < 108);
        assert_eq!(fs::metadata(&ea.path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(ea.path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        drop((a, b));
        assert!(!ea.path.exists() && !eb.path.exists());
    }

    #[test]
    fn absent_and_refused_requests_are_none_without_cleanup_or_creation() {
        let state = tempfile::tempdir().unwrap();
        let absent = state.path().join("absent");
        assert!(request(&absent, Command::Status).unwrap().is_none());
        assert!(!absent.exists());
        let ep = endpoint(state.path());
        assert!(request(state.path(), Command::Status).unwrap().is_none());
        stale(&ep.path);
        assert!(request(state.path(), Command::Show).unwrap().is_none());
        assert!(ep.path.exists());
        let server = spawn(state.path(), callback()).unwrap();
        assert!(request(state.path(), Command::Show).unwrap().is_some());
        drop(server);
        assert!(!ep.path.exists());
    }

    #[test]
    fn live_socket_is_never_unlinked_or_taken_over() {
        let state = tempfile::tempdir().unwrap();
        let server = spawn(state.path(), callback()).unwrap();
        let ep = endpoint(state.path());
        let inode = fs::symlink_metadata(&ep.path).unwrap().ino();
        assert!(spawn(state.path(), callback()).is_err());
        assert_eq!(fs::symlink_metadata(&ep.path).unwrap().ino(), inode);
        assert!(request(state.path(), Command::Show).unwrap().is_some());
        drop(server);
    }

    #[test]
    fn publication_is_preconfigured_and_never_replaces_an_existing_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let temporary = directory.path().join("temporary.sock");
        let destination = directory.path().join("published.sock");
        let _listener = StdListener::bind(&temporary).unwrap();
        let mut owned = OwnedSocket::inspect(&temporary).unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&destination, b"preserve").unwrap();
        assert!(owned.publish(&destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"preserve");
        assert!(temporary.exists());
        fs::remove_file(&destination).unwrap();
        owned.publish(&destination).unwrap();
        assert!(!temporary.exists());
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o777, 0o600);
        let _client = StdStream::connect(&destination).unwrap();
        drop(owned);
        assert!(!destination.exists());
    }

    #[test]
    fn wrong_file_symlink_and_insecure_socket_are_never_removed() {
        let state = tempfile::tempdir().unwrap();
        let ep = endpoint(state.path());
        fs::write(&ep.path, b"preserve").unwrap();
        assert!(spawn(state.path(), callback()).is_err());
        assert!(request(state.path(), Command::Show).is_err());
        assert_eq!(fs::read(&ep.path).unwrap(), b"preserve");
        fs::remove_file(&ep.path).unwrap();
        let target = state.path().join("target");
        fs::write(&target, b"target").unwrap();
        symlink(&target, &ep.path).unwrap();
        assert!(spawn(state.path(), callback()).is_err());
        assert!(request(state.path(), Command::Show).is_err());
        assert!(
            fs::symlink_metadata(&ep.path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"target");
        fs::remove_file(&ep.path).unwrap();
        stale(&ep.path);
        fs::set_permissions(&ep.path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(spawn(state.path(), callback()).is_err());
        assert!(request(state.path(), Command::Status).is_err());
        assert!(ep.path.exists());
        fs::remove_file(&ep.path).unwrap();
    }

    #[test]
    fn cleanup_does_not_remove_replacement_file_or_socket() {
        for replace_with_socket in [false, true] {
            let state = tempfile::tempdir().unwrap();
            let server = spawn(state.path(), callback()).unwrap();
            let ep = endpoint(state.path());
            let moved = ep.path.with_extension("old");
            fs::rename(&ep.path, &moved).unwrap();
            if replace_with_socket {
                stale(&ep.path);
            } else {
                fs::write(&ep.path, b"replacement").unwrap();
            }
            let before = fs::symlink_metadata(&ep.path).unwrap().ino();
            drop(server);
            assert_eq!(fs::symlink_metadata(&ep.path).unwrap().ino(), before);
            fs::remove_file(&ep.path).unwrap();
            fs::remove_file(&moved).unwrap();
        }
    }

    #[test]
    fn malformed_mismatched_and_oversize_requests_never_reach_callback() {
        let state = tempfile::tempdir().unwrap();
        let called = Arc::new(AtomicUsize::new(0));
        let count = called.clone();
        let _server = spawn(
            state.path(),
            Arc::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Null)
            }),
        )
        .unwrap();
        let ep = endpoint(state.path());
        for bytes in [
            b"not-json\n".to_vec(),
            line(&Message {
                state: "different-state".into(),
                command: Command::Quit,
            })
            .unwrap(),
            vec![b'x'; LIMIT],
        ] {
            assert!(matches!(raw(&ep.path, &bytes), Reply::Error(_)));
        }
        assert_eq!(called.load(Ordering::SeqCst), 0);
        assert_eq!(
            request(state.path(), Command::Status).unwrap(),
            Some(Value::Null)
        );
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn callback_errors_and_oversize_responses_are_errors_not_missing_instances() {
        for large in [false, true] {
            let state = tempfile::tempdir().unwrap();
            let _server = spawn(
                state.path(),
                Arc::new(move |_| {
                    if large {
                        Ok(Value::String("x".repeat(LIMIT)))
                    } else {
                        anyhow::bail!("window unavailable")
                    }
                }),
            )
            .unwrap();
            let error = request(state.path(), Command::Show)
                .unwrap_err()
                .to_string();
            assert!(error.contains(if large {
                "exceeds 16 KiB"
            } else {
                "window unavailable"
            }));
        }
    }

    #[test]
    fn unresponsive_server_times_out_without_removing_socket() {
        let state = tempfile::tempdir().unwrap();
        let ep = endpoint(state.path());
        let _listener = StdListener::bind(&ep.path).unwrap();
        fs::set_permissions(&ep.path, fs::Permissions::from_mode(0o600)).unwrap();
        let before = Instant::now();
        let error = request(state.path(), Command::Status)
            .unwrap_err()
            .to_string();
        assert!(error.contains("timed out"));
        assert!(before.elapsed() < Duration::from_secs(3));
        assert!(ep.path.exists());
        fs::remove_file(&ep.path).unwrap();
    }

    #[test]
    fn drop_interrupts_partial_requests_and_private_directory_rejects_symlinks() {
        let state = tempfile::tempdir().unwrap();
        let server = spawn(state.path(), callback()).unwrap();
        let ep = endpoint(state.path());
        let mut partial = StdStream::connect(&ep.path).unwrap();
        partial.write_all(b"{").unwrap();
        let before = Instant::now();
        drop(server);
        assert!(before.elapsed() < Duration::from_secs(1));
        assert!(!ep.path.exists());
        let bad = state.path().join("symlink-runtime");
        symlink(state.path(), &bad).unwrap();
        assert!(private_directory(&bad, true).is_err());
        let insecure = state.path().join("insecure-runtime");
        fs::create_dir(&insecure).unwrap();
        fs::set_permissions(&insecure, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(private_directory(&insecure, true).is_err());
        assert_eq!(fs::metadata(&insecure).unwrap().mode() & 0o777, 0o777);
    }
}
