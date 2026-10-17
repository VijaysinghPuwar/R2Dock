//! Development helper: run a local S3-compatible server for manual testing.
//!
//! `cargo run -p r2dock-testkit --bin local-s3 -- <port> <bucket>...`
//! Credentials: see `ACCESS_KEY` and `SECRET_KEY` in the testkit library.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(9300);
    let buckets: Vec<String> = args.collect();
    let root = std::env::temp_dir().join(format!("r2dock-local-s3-{port}"));
    for b in &buckets {
        std::fs::create_dir_all(root.join(b)).unwrap();
    }
    let fs = s3s_fs::FileSystem::new(&root).unwrap();
    let service = {
        let mut b = S3ServiceBuilder::new(fs);
        b.set_auth(SimpleAuth::from_single(
            r2dock_testkit::ACCESS_KEY,
            r2dock_testkit::SECRET_KEY,
        ));
        b.build()
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    println!(
        "local S3 listening on http://127.0.0.1:{port} (data in {})",
        root.display()
    );
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
}
