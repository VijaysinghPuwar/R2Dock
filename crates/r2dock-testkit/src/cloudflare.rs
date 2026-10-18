//! A tiny stand-in for the Cloudflare API bucket domain endpoints, so tests
//! never call the real API.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The documented "list custom domains" example, with fictional names.
pub const CUSTOM_DOMAINS_OK: &str = r#"{
  "errors": [], "messages": [], "success": true,
  "result": { "domains": [
    { "domain": "prefix.example-domain.one.com", "enabled": false,
      "status": { "ownership": "deactivated", "ssl": "pending" },
      "ciphers": ["<string>"], "minTLS": "1.0", "zoneId": "00000000000000000000000000000000",
      "zoneName": "example-domain.one.com" },
    { "domain": "media.example.com", "enabled": true,
      "status": { "ownership": "active", "ssl": "active" },
      "minTLS": "1.2", "zoneName": "example.com" }
  ] }
}"#;

/// No custom domains.
pub const CUSTOM_DOMAINS_EMPTY: &str =
    r#"{"errors":[],"messages":[],"success":true,"result":{"domains":[]}}"#;

/// The documented "get managed domain" example, enabled.
pub const MANAGED_DOMAIN_OK: &str = r#"{
  "errors": [], "messages": [], "success": true,
  "result": { "bucketId": "00000000000000000000000000000000",
              "domain": "pub-00000000000000000000000000000000.r2.dev", "enabled": true }
}"#;

/// An authentication error body as the API returns it.
pub const AUTH_ERROR: &str = r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}],"messages":[],"result":null}"#;

#[derive(Debug, Clone, Copy)]
pub enum Reply {
    Json(u16, &'static str),
    /// Accept the request and never answer.
    Hang,
}

/// A running mock API. Dropping it stops accepting connections.
pub struct MockCloudflare {
    /// Use as the API base URL (ends in `/client/v4`).
    pub base: String,
    seen: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockCloudflare {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockCloudflare {
    /// Serve `custom` for `.../domains/custom` and `managed` for every other path.
    pub async fn start(custom: Reply, managed: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_c = seen.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let seen = seen_c.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_string();
                    seen.lock().unwrap().push(head.clone());
                    let reply = if head.contains("/domains/custom ") {
                        custom
                    } else {
                        managed
                    };
                    match reply {
                        Reply::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
                        Reply::Json(status, body) => {
                            let text = format!(
                                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = sock.write_all(text.as_bytes()).await;
                        }
                    }
                });
            }
        });
        Self {
            base: format!("http://{addr}/client/v4"),
            seen,
            task,
        }
    }

    /// The request heads received so far.
    pub fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}
