//! Test-only helpers. Starts an in-process S3-compatible server backed by a
//! temporary folder, so integration tests never touch a real bucket.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use tokio::net::TcpListener;

pub const ACCESS_KEY: &str = "testaccesskey";
pub const SECRET_KEY: &str = "testsecretkey-0123456789abcdef";

/// A running local S3 server. Dropping it stops accepting connections.
pub struct LocalS3 {
    pub endpoint: String,
    pub root: PathBuf,
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
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let http = ConnBuilder::new(TokioExecutor::new());
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    continue;
                };
                let conn = http
                    .serve_connection(TokioIo::new(socket), service.clone())
                    .into_owned();
                tokio::spawn(async move {
                    let _ = conn.await;
                });
            }
        });
        Self {
            endpoint: format!("http://{addr}"),
            root,
            _dir: dir,
            task,
        }
    }

    /// Path of an object's data file on disk (for corruption tests).
    pub fn object_path(&self, bucket: &str, key: &str) -> PathBuf {
        self.root.join(bucket).join(key)
    }
}
