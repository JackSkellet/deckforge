//! Public guild-widget counts; only called after explicit Connect/authentication.
use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub async fn presence(client: &reqwest::Client, base: &str, guild_id: &str) -> Result<u64> {
    ensure!(
        !guild_id.is_empty()
            && guild_id.len() <= 22
            && guild_id.bytes().all(|b| b.is_ascii_digit()),
        "Invalid guild widget ID"
    );
    let mut response = client
        .get(format!(
            "{}/{guild_id}/widget.json",
            base.trim_end_matches('/')
        ))
        .send()
        .await
        .context("Guild widget is unavailable")?;
    ensure!(
        response.status().is_success(),
        "Guild widget is disabled or unavailable (HTTP {})",
        response.status().as_u16()
    );
    ensure!(
        response.content_length().unwrap_or(0) <= 1024 * 1024,
        "Guild widget response exceeds 1 MiB"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "Guild widget response exceeds 1 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).context("Invalid guild widget response")?;
    ensure!(
        value["id"].as_str() == Some(guild_id),
        "Guild widget identity mismatch"
    );
    value["presence_count"]
        .as_u64()
        .context("Guild widget has no online count")
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    #[tokio::test]
    async fn widget_uses_public_get_and_checks_identity() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/guilds", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for id in ["123", "456"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![];
                loop {
                    let mut buffer = [0; 1024];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("GET /guilds/123/widget.json "));
                assert!(!request.to_lowercase().contains("authorization:"));
                let body = format!(r#"{{"id":"{id}","presence_count":7}}"#);
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = reqwest::Client::new();
        assert_eq!(presence(&client, &base, "123").await.unwrap(), 7);
        assert!(presence(&client, &base, "123").await.is_err());
        server.await.unwrap();
        assert!(presence(&client, &base, "../123").await.is_err());
    }
}
