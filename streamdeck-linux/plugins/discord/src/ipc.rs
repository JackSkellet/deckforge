use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::{mpsc, oneshot},
};

pub const MAX_FRAME: usize = 1024 * 1024;
pub enum Event {
    Connected,
    Frame(Value),
    Offline(String),
}
pub enum Command {
    Frame(Value),
    Flush(oneshot::Sender<()>),
}
struct AbortReader(tokio::task::JoinHandle<()>);
impl Drop for AbortReader {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub fn paths() -> Vec<PathBuf> {
    let base = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|key| std::env::var_os(key).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    (0..10)
        .map(|index| base.join(format!("discord-ipc-{index}")))
        .collect()
}
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<(u32, Vec<u8>)> {
    let mut header = [0; 8];
    reader.read_exact(&mut header).await?;
    let opcode = u32::from_le_bytes(header[..4].try_into()?);
    let length = u32::from_le_bytes(header[4..].try_into()?) as usize;
    ensure!(length <= MAX_FRAME, "Discord IPC frame exceeds 1 MiB");
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).await?;
    Ok((opcode, payload))
}
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    opcode: u32,
    bytes: &[u8],
) -> Result<()> {
    ensure!(bytes.len() <= MAX_FRAME, "Discord IPC frame exceeds 1 MiB");
    writer.write_all(&opcode.to_le_bytes()).await?;
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}
async fn session(
    stream: UnixStream,
    client_id: &str,
    commands: &mut mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
) -> Result<()> {
    let (mut reader, mut writer) = stream.into_split();
    write_frame(
        &mut writer,
        0,
        &serde_json::to_vec(&json!({"v":1,"client_id":client_id}))?,
    )
    .await?;
    events
        .send(Event::Connected)
        .await
        .context("Plugin event loop closed")?;
    let (frames, mut incoming) = mpsc::channel(8);
    let _reader = AbortReader(tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut reader).await;
            let failed = frame.is_err();
            if frames.send(frame).await.is_err() || failed {
                return;
            }
        }
    }));
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                match command {
                    Command::Frame(value) => write_frame(&mut writer, 1, &serde_json::to_vec(&value)?).await?,
                    Command::Flush(done) => { writer.flush().await?; let _ = done.send(()); }
                }
            }
            frame = incoming.recv() => {
                let (opcode, bytes) = frame.context("Discord reader stopped")??;
                match opcode {
                    1 => { events.send(Event::Frame(serde_json::from_slice(&bytes).context("Invalid Discord IPC JSON")?)).await.context("Plugin event loop closed")?; }
                    2 => anyhow::bail!("Discord closed the IPC session"),
                    3 => write_frame(&mut writer, 4, &bytes).await?,
                    4 => {},
                    _ => anyhow::bail!("Unsupported Discord IPC opcode"),
                }
            }
        }
    }
}
// Tasks start only after an explicit Connect or a previously enabled session. Aborting this task drops
// its socket, pending frame and retry sleep; reconnect cannot outlive Disconnect.
pub async fn supervise(
    paths: Vec<PathBuf>,
    client_id: String,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
) {
    let mut delay = 1;
    loop {
        if commands.is_closed() {
            return;
        }
        let mut connected = None;
        for path in &paths {
            if let Ok(Ok(stream)) =
                tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(path)).await
            {
                // Never send authorization to another user's socket in a shared
                // runtime directory such as /tmp. Check the connected peer,
                // not a path that could have been replaced after inspection.
                if stream
                    .peer_cred()
                    .is_ok_and(|peer| peer.uid() == unsafe { libc::geteuid() })
                {
                    connected = Some(stream);
                    break;
                }
            }
        }
        let message = if let Some(stream) = connected {
            delay = 1;
            match session(stream, &client_id, &mut commands, &events).await {
                Ok(()) => return,
                Err(error) => error.to_string(),
            }
        } else {
            "Discord desktop IPC is unavailable; waiting for the app".into()
        };
        while commands.try_recv().is_ok() {}
        if events.send(Event::Offline(message)).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(30);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fragmented_frames_and_limit() {
        let (mut a, mut b) = tokio::io::duplex(64);
        let task = tokio::spawn(async move {
            for byte in [1, 0, 0, 0, 2, 0, 0, 0, b'{', b'}'] {
                a.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert_eq!(read_frame(&mut b).await.unwrap(), (1, b"{}".to_vec()));
        task.await.unwrap();
        let (mut a, mut b) = tokio::io::duplex(16);
        a.write_all(&[1, 0, 0, 0, 255, 255, 255, 127])
            .await
            .unwrap();
        assert!(
            read_frame(&mut b)
                .await
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
    }
    #[tokio::test]
    async fn fake_unix_peer_handshake_ping_and_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("discord-ipc-0");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (tx, rx) = mpsc::channel(8);
        let (events, mut received) = mpsc::channel(8);
        let task = tokio::spawn(supervise(
            vec![path],
            "123456789012345678".into(),
            rx,
            events,
        ));
        let (mut peer, _) = listener.accept().await.unwrap();
        let (op, data) = read_frame(&mut peer).await.unwrap();
        assert_eq!(op, 0);
        assert_eq!(serde_json::from_slice::<Value>(&data).unwrap()["v"], 1);
        assert!(matches!(received.recv().await, Some(Event::Connected)));
        write_frame(&mut peer, 3, b"fixture-ping").await.unwrap();
        assert_eq!(
            read_frame(&mut peer).await.unwrap(),
            (4, b"fixture-ping".to_vec())
        );
        tx.send(Command::Frame(
            json!({"cmd":"GET_GUILDS","nonce":"fixture"}),
        ))
        .await
        .unwrap();
        let (_, data) = read_frame(&mut peer).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&data).unwrap()["nonce"],
            "fixture"
        );
        // An outbound command must not cancel a partially read inbound header.
        peer.write_all(&[1, 0, 0, 0]).await.unwrap();
        tx.send(Command::Frame(
            json!({"cmd":"GET_CHANNELS","nonce":"during-read"}),
        ))
        .await
        .unwrap();
        let (_, bytes) = read_frame(&mut peer).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["nonce"],
            "during-read"
        );
        let payload = br#"{"cmd":"DISPATCH","evt":"READY","data":{}}"#;
        peer.write_all(&(payload.len() as u32).to_le_bytes())
            .await
            .unwrap();
        peer.write_all(payload).await.unwrap();
        assert!(
            matches!(received.recv().await, Some(Event::Frame(value)) if value["evt"] == "READY")
        );
        drop(peer);
        assert!(matches!(received.recv().await, Some(Event::Offline(_))));
        let (mut second, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read_frame(&mut second).await.unwrap().0, 0);
        task.abort();
        let _ = task.await;
    }
}
