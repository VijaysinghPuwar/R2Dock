//! Test-only helpers. Starts an in-process S3-compatible server backed by a
//! temporary folder, so integration tests never touch a real bucket.
//!
//! [`Network`] shapes the server's link: a shared upload bandwidth limit and
//! a delay before multipart aborts are answered, to test behavior on slow or
//! flaky networks without leaving the machine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod cloudflare;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use tokio::net::TcpListener;
use tokio::time::Instant;

pub const ACCESS_KEY: &str = "testaccesskey";
pub const SECRET_KEY: &str = "testsecretkey-0123456789abcdef";

/// Shapes the local server's network. Clones share the same settings, so a
/// test can change them while the server runs.
#[derive(Clone, Default)]
pub struct Network {
    inner: Arc<NetworkInner>,
}

#[derive(Default)]
struct NetworkInner {
    /// Request body bytes per second, shared by all requests (0 = no limit).
    upload_bps: AtomicU64,
    /// Delay before a multipart abort is answered, in milliseconds.
    abort_delay_ms: AtomicU64,
    /// Multipart aborts received.
    aborts: AtomicU64,
    /// When the shared link is free again.
    next_free: Mutex<Option<Instant>>,
}

impl Network {
    /// Limit request bodies to `bps` bytes per second in total (0 = no limit).
    pub fn set_upload_bps(&self, bps: u64) {
        self.inner.upload_bps.store(bps, Ordering::Relaxed);
    }

    /// Answer multipart aborts only after `d` (a slow or flaky network).
    pub fn set_abort_delay(&self, d: Duration) {
        let ms = u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        self.inner.abort_delay_ms.store(ms, Ordering::Relaxed);
    }

    /// Multipart aborts the server has received so far.
    pub fn aborts(&self) -> u64 {
        self.inner.aborts.load(Ordering::Relaxed)
    }

    /// Reserve `d` of shared link time; returns when the reservation ends.
    fn reserve(&self, d: Duration) -> Instant {
        let mut next = self
            .inner
            .next_free
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let start = next.filter(|t| *t > now).unwrap_or(now);
        let end = start + d;
        *next = Some(end);
        end
    }

    /// Delay a multipart abort when asked to.
    async fn before(&self, req: &hyper::Request<hyper::body::Incoming>) {
        let is_abort = req.method() == hyper::Method::DELETE
            && req.uri().query().is_some_and(|q| q.contains("uploadId="));
        if is_abort {
            self.inner.aborts.fetch_add(1, Ordering::Relaxed);
            let ms = self.inner.abort_delay_ms.load(Ordering::Relaxed);
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
        }
    }

    /// The request body, read no faster than the shared upload limit.
    fn body(&self, body: hyper::body::Incoming) -> s3s::Body {
        if self.inner.upload_bps.load(Ordering::Relaxed) == 0 {
            return s3s::Body::from(body);
        }
        s3s::Body::http_body(Throttled {
            inner: body,
            net: self.clone(),
            held: None,
            wait: None,
        })
    }
}

/// A request body that hands out each chunk only once the shared link has
/// had time to carry it. Keeps the size hint, which S3 handlers need.
struct Throttled {
    inner: hyper::body::Incoming,
    net: Network,
    held: Option<http_body::Frame<bytes::Bytes>>,
    wait: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}

impl http_body::Body for Throttled {
    type Data = bytes::Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        use std::task::Poll;
        let this = &mut *self;
        if let Some(wait) = &mut this.wait {
            if wait.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            this.wait = None;
            return Poll::Ready(this.held.take().map(Ok));
        }
        let frame = match std::pin::Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(f))) => f,
            other => return other,
        };
        let bps = this.net.inner.upload_bps.load(Ordering::Relaxed);
        let len = frame.data_ref().map_or(0, bytes::Bytes::len);
        if bps == 0 || len == 0 {
            return Poll::Ready(Some(Ok(frame)));
        }
        #[allow(clippy::cast_precision_loss)]
        let d = Duration::from_secs_f64(len as f64 / bps as f64);
        let mut wait = Box::pin(tokio::time::sleep_until(this.net.reserve(d)));
        if wait.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Ok(frame)));
        }
        this.held = Some(frame);
        this.wait = Some(wait);
        Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        self.held.is_none() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        let mut hint = self.inner.size_hint();
        if let Some(n) = self
            .held
            .as_ref()
            .and_then(|f| f.data_ref())
            .map(bytes::Bytes::len)
        {
            let n = n as u64;
            let lower = hint.lower() + n;
            if let Some(u) = hint.upper() {
                hint.set_upper(u + n);
            }
            hint.set_lower(lower);
        }
        hint
    }
}

/// A running local S3 server. Dropping it stops accepting connections.
pub struct LocalS3 {
    pub endpoint: String,
    pub root: PathBuf,
    /// Shapes this server's link (no limits by default).
    pub network: Network,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for LocalS3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LocalS3 {
    /// Start a server with the given buckets created.
    pub async fn start(buckets: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for b in buckets {
            std::fs::create_dir_all(root.join(b)).unwrap();
        }
        let fs = s3s_fs::FileSystem::new(&root).unwrap();
        let service = {
            let mut b = S3ServiceBuilder::new(fs);
            b.set_auth(SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
            b.build()
        };
        let network = Network::default();
        let shaping = network.clone();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let http = ConnBuilder::new(TokioExecutor::new());
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    continue;
                };
                let service = service.clone();
                let network = shaping.clone();
                let svc = hyper::service::service_fn(
                    move |req: hyper::Request<hyper::body::Incoming>| {
                        let service = service.clone();
                        let network = network.clone();
                        async move {
                            network.before(&req).await;
                            let req = req.map(|b| network.body(b));
                            service.call(req).await
                        }
                    },
                );
                let conn = http
                    .serve_connection(TokioIo::new(socket), svc)
                    .into_owned();
                tokio::spawn(async move {
                    let _ = conn.await;
                });
            }
        });
        Self {
            endpoint: format!("http://{addr}"),
            root,
            network,
            _dir: dir,
            task,
        }
    }

    /// Path of an object's data file on disk (for corruption tests).
    pub fn object_path(&self, bucket: &str, key: &str) -> PathBuf {
        self.root.join(bucket).join(key)
    }
}
