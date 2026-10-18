//! A tiny in-process S3 mock for request-ordering tests.
//!
//! It understands just enough of the S3 protocol for copy, move, and
//! multipart copy: HEAD, GET-less object model, CopyObject, DeleteObject,
//! CreateMultipartUpload, UploadPartCopy, CompleteMultipartUpload, and
//! AbortMultipartUpload. Authentication is ignored. A hook runs after each
//! request is applied and before the response is sent, so tests can change
//! objects at exact request boundaries.
//!
//! Conditional headers follow the R2 documentation: `x-amz-copy-source-if-match`
//! and `cf-copy-destination-if-none-match` are honored on CopyObject; the
//! source condition on UploadPartCopy is listed as unsupported by R2 and is
//! ignored here; DeleteObject and CompleteMultipartUpload take no conditions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Obj {
    pub body: Vec<u8>,
    /// Reported size; may exceed `body.len()` to fake very large objects.
    pub size: u64,
    pub etag: String,
    /// Extra response headers for HEAD (lowercase names).
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub key: String,
    pub query: String,
    /// Lowercase header names.
    pub headers: Vec<(String, String)>,
    pub status: u16,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn is(&self, method: &str, key: &str) -> bool {
        self.method == method && self.key == key
    }
}

#[derive(Debug, Default)]
pub struct Model {
    pub objects: BTreeMap<String, Obj>,
    pub requests: Vec<Req>,
    /// Answer every DeleteObject with a server error.
    pub fail_deletes: bool,
    /// Apply every CopyObject but answer with a server error, as when the
    /// response is lost after the copy happened.
    pub lose_copy_responses: bool,
    next_version: u64,
    uploads: BTreeMap<String, (String, Vec<(String, String)>)>,
}

impl Model {
    /// Store an object with a fresh ETag, like any write would.
    pub fn put(&mut self, key: &str, body: &[u8]) {
        self.put_sized(key, body, body.len() as u64, Vec::new());
    }

    pub fn put_sized(&mut self, key: &str, body: &[u8], size: u64, headers: Vec<(String, String)>) {
        let etag = self.fresh_etag();
        self.objects.insert(
            key.to_string(),
            Obj {
                body: body.to_vec(),
                size,
                etag,
                headers,
            },
        );
    }

    pub fn body(&self, key: &str) -> Option<&[u8]> {
        self.objects.get(key).map(|o| o.body.as_slice())
    }

    /// `METHOD key` for every request, in order.
    pub fn sequence(&self) -> Vec<String> {
        self.requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.key))
            .collect()
    }

    fn fresh_etag(&mut self) -> String {
        self.next_version += 1;
        format!("\"v{}\"", self.next_version)
    }
}

pub type Hook = Arc<dyn Fn(&Req, &mut Model) + Send + Sync>;

pub struct MockS3 {
    pub endpoint: String,
    pub model: Arc<Mutex<Model>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockS3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockS3 {
    pub async fn start(model: Model, hook: Hook) -> Self {
        let model = Arc::new(Mutex::new(model));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = model.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    continue;
                };
                let state = state.clone();
                let hook = hook.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, state, hook).await;
                });
            }
        });
        Self {
            endpoint,
            model,
            task,
        }
    }
}

async fn serve(
    socket: tokio::net::TcpStream,
    state: Arc<Mutex<Model>>,
    hook: Hook,
) -> std::io::Result<()> {
    let (read, mut write) = socket.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let target = parts.next().unwrap_or_default().to_string();
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).await?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let find = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        if find("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
            write.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }
        let len: usize = find("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;

        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let key = path
            .trim_start_matches('/')
            .split_once('/')
            .map_or("", |(_, k)| k)
            .to_string();
        let mut req = Req {
            method,
            key,
            query: query.to_string(),
            headers,
            status: 0,
        };
        let (status, extra, payload) = {
            let mut m = state.lock().unwrap();
            let (status, extra, payload) = handle(&mut m, &req);
            req.status = status;
            m.requests.push(req.clone());
            hook(&req, &mut m);
            (status, extra, payload)
        };
        let mut head = format!("HTTP/1.1 {status} X\r\n");
        let declared = extra
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
        for (k, v) in &extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if !declared {
            head.push_str(&format!("content-length: {}\r\n", payload.len()));
        }
        head.push_str("\r\n");
        write.write_all(head.as_bytes()).await?;
        if req.method != "HEAD" {
            write.write_all(payload.as_bytes()).await?;
        }
        write.flush().await?;
    }
}

type Response = (u16, Vec<(String, String)>, String);

fn xml(status: u16, body: String) -> Response {
    (
        status,
        vec![("content-type".into(), "application/xml".into())],
        body,
    )
}

fn error(status: u16, code: &str) -> Response {
    xml(
        status,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{code}</Message></Error>"
        ),
    )
}

fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == name).then_some(v)
    })
}

fn copy_source_key(req: &Req) -> Option<String> {
    let src = req.header("x-amz-copy-source")?;
    let src = src.trim_start_matches('/');
    Some(src.split_once('/').map_or("", |(_, k)| k).to_string())
}

fn handle(m: &mut Model, req: &Req) -> Response {
    let key = req.key.clone();
    match req.method.as_str() {
        "HEAD" => match m.objects.get(&key) {
            None => (404, Vec::new(), String::new()),
            Some(o) => {
                let mut h = vec![
                    ("content-length".to_string(), o.size.to_string()),
                    ("etag".to_string(), o.etag.clone()),
                    (
                        "last-modified".to_string(),
                        "Sun, 18 Oct 2026 00:00:00 GMT".to_string(),
                    ),
                ];
                h.extend(o.headers.iter().cloned());
                (200, h, String::new())
            }
        },
        "DELETE" if param(&req.query, "uploadId").is_some() => {
            let id = param(&req.query, "uploadId")
                .unwrap_or_default()
                .to_string();
            m.uploads.remove(&id);
            (204, Vec::new(), String::new())
        }
        "DELETE" if m.fail_deletes => error(500, "InternalError"),
        "DELETE" => {
            m.objects.remove(&key);
            (204, Vec::new(), String::new())
        }
        "POST" if param(&req.query, "uploads").is_some() => {
            let id = format!("upload-{}", m.uploads.len() + 1);
            m.uploads
                .insert(id.clone(), (key.clone(), req.headers.clone()));
            xml(
                200,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><InitiateMultipartUploadResult><Bucket>bkt</Bucket><Key>{key}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>"
                ),
            )
        }
        "POST" if param(&req.query, "uploadId").is_some() => {
            let id = param(&req.query, "uploadId")
                .unwrap_or_default()
                .to_string();
            let Some((dst, _headers)) = m.uploads.remove(&id) else {
                return error(404, "NoSuchUpload");
            };
            // The mock keeps the source bytes on the upload; good enough for
            // ordering tests. The completed object gets a fresh ETag.
            let source = m
                .requests
                .iter()
                .rev()
                .find(|r| r.method == "PUT" && param(&r.query, "uploadId") == Some(id.as_str()))
                .and_then(copy_source_key)
                .and_then(|k| m.objects.get(&k).cloned());
            let etag = format!("\"mpu-{}-2\"", m.requests.len());
            let (body, size) = source.map_or((Vec::new(), 0), |o| (o.body, o.size));
            m.objects.insert(
                dst.clone(),
                Obj {
                    body,
                    size,
                    etag: etag.clone(),
                    headers: Vec::new(),
                },
            );
            xml(
                200,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUploadResult><Location>x</Location><Bucket>bkt</Bucket><Key>{dst}</Key><ETag>{}</ETag></CompleteMultipartUploadResult>",
                    xml_escape(&etag)
                ),
            )
        }
        "PUT" if param(&req.query, "uploadId").is_some() => {
            // UploadPartCopy. R2 lists x-amz-copy-source-if-match as
            // unsupported here, so it is not evaluated.
            let Some(src) = copy_source_key(req) else {
                return error(400, "InvalidRequest");
            };
            if !m.objects.contains_key(&src) {
                return error(404, "NoSuchKey");
            }
            let n = param(&req.query, "partNumber").unwrap_or("0");
            xml(
                200,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyPartResult><ETag>&quot;part-{n}&quot;</ETag><LastModified>2026-10-18T00:00:00.000Z</LastModified></CopyPartResult>"
                ),
            )
        }
        "PUT" => {
            let Some(src) = copy_source_key(req) else {
                return error(501, "NotImplemented");
            };
            let Some(source) = m.objects.get(&src).cloned() else {
                return error(404, "NoSuchKey");
            };
            if let Some(want) = req.header("x-amz-copy-source-if-match")
                && want != source.etag
            {
                return error(412, "PreconditionFailed");
            }
            if req.header("cf-copy-destination-if-none-match") == Some("*")
                && m.objects.contains_key(&key)
            {
                return error(412, "PreconditionFailed");
            }
            let etag = m.fresh_etag();
            m.objects.insert(
                key,
                Obj {
                    etag: etag.clone(),
                    ..source
                },
            );
            if m.lose_copy_responses {
                return error(500, "InternalError");
            }
            xml(
                200,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyObjectResult><ETag>{}</ETag><LastModified>2026-10-18T00:00:00.000Z</LastModified></CopyObjectResult>",
                    xml_escape(&etag)
                ),
            )
        }
        _ => error(501, "NotImplemented"),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}
