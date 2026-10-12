use crate::config::{Config, Token, now};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

#[derive(Deserialize)]
struct Response {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    expires_in: u64,
}
pub async fn exchange(
    client: &reqwest::Client,
    endpoint: &str,
    config: &Config,
    code: &str,
    refresh: bool,
) -> Result<Token> {
    let mut fields = vec![
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        (
            "grant_type",
            if refresh {
                "refresh_token"
            } else {
                "authorization_code"
            },
        ),
        (if refresh { "refresh_token" } else { "code" }, code),
    ];
    if !config.redirect_uri.is_empty() {
        fields.push(("redirect_uri", config.redirect_uri.as_str()));
    }
    let response = client
        .post(endpoint)
        .form(&fields)
        .send()
        .await
        .context("Discord OAuth token endpoint is unavailable")?;
    // Error bodies can contain sensitive values. Neither log nor forward them.
    ensure!(
        response.status().is_success(),
        "Discord OAuth token exchange rejected (HTTP {})",
        response.status().as_u16()
    );
    ensure!(
        response.content_length().unwrap_or(0) <= 65536,
        "Discord OAuth response exceeds 64 KiB"
    );
    let mut bytes = Vec::new();
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 65536,
            "Discord OAuth response exceeds 64 KiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let token: Response = serde_json::from_slice(&bytes)
        .context("Discord OAuth returned an invalid token response")?;
    ensure!(
        !token.access_token.is_empty()
            && token.access_token.len() <= 8192
            && token.refresh_token.len() <= 8192,
        "Discord OAuth returned invalid token fields"
    );
    Ok(Token {
        access_token: token.access_token,
        refresh_token: if token.refresh_token.is_empty() && refresh {
            code.to_owned()
        } else {
            token.refresh_token
        },
        expires_at: now().saturating_add(token.expires_in.min(365 * 24 * 3600)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    #[tokio::test]
    async fn standard_oauth_form_exchange_against_local_fake_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(at) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..at]);
                    let length: usize = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= at + 4 + length {
                        let body = String::from_utf8_lossy(&bytes[at + 4..]);
                        assert!(body.contains("grant_type=authorization_code"));
                        assert!(body.contains("code=fixture-code"));
                        assert!(body.contains("client_secret=fixture-secret"));
                        break;
                    }
                }
            }
            let body = r#"{"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        });
        let config = Config {
            client_id: "123456789012345678".into(),
            client_secret: "fixture-secret".into(),
            redirect_uri: String::new(),
            scopes: vec!["rpc".into()],
            token: None,
        };
        let token = exchange(
            &reqwest::Client::new(),
            &endpoint,
            &config,
            "fixture-code",
            false,
        )
        .await
        .unwrap();
        assert_eq!(token.access_token, "fixture-access");
        assert!(token.expires_at > now());
        server.await.unwrap();
    }
}
