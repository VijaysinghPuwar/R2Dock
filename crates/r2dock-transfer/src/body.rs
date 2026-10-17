//! Streaming request bodies read from disk with progress counting.
//!
//! Each body re-opens the source file (with an identity check) and reads one
//! byte range in 64 KiB chunks. Bodies are rebuildable so SDK retries work,
//! and a rebuild resets the in-flight count so progress never double counts.

use std::io::SeekFrom;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::StreamBody;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::progress::Counter;
use crate::source::SourceFile;

const CHUNK: usize = 64 * 1024;

/// Build a retryable body for `len` bytes starting at `offset`.
///
/// `sent_this_attempt` tracks bytes counted as in flight by the current
/// attempt. When the request ends, the caller removes them from the counter
/// (and commits `len` on success): see [`settle`].
pub fn range_body(
    source: Arc<SourceFile>,
    offset: u64,
    len: u64,
    counter: Arc<Counter>,
    sent_this_attempt: Arc<AtomicU64>,
) -> ByteStream {
    let body = SdkBody::retryable(move || {
        // A rebuild starts a new attempt: drop what the previous one counted.
        let previous = sent_this_attempt.swap(0, Ordering::Relaxed);
        counter.sub_in_flight(previous);
        match source.reopen() {
            Ok(file) => {
                let counter = counter.clone();
                let sent = sent_this_attempt.clone();
                let stream = futures_util::stream::once(async move {
                    let mut file = tokio::fs::File::from_std(file);
                    file.seek(SeekFrom::Start(offset)).await?;
                    Ok::<_, std::io::Error>(ReaderStream::with_capacity(file.take(len), CHUNK))
                })
                .map(|r| match r {
                    Ok(s) => s.left_stream(),
                    Err(e) => futures_util::stream::once(async move { Err(e) }).right_stream(),
                })
                .flatten()
                .map(move |chunk: Result<Bytes, std::io::Error>| {
                    chunk.map(|b| {
                        let n = b.len() as u64;
                        counter.add_in_flight(n);
                        sent.fetch_add(n, Ordering::Relaxed);
                        Frame::data(b)
                    })
                });
                SdkBody::from_body_1_x(StreamBody::new(stream))
            }
            Err(_) => {
                let stream = futures_util::stream::once(async {
                    Err::<Frame<Bytes>, std::io::Error>(std::io::Error::other(
                        "source file changed",
                    ))
                });
                SdkBody::from_body_1_x(StreamBody::new(stream))
            }
        }
    });
    ByteStream::new(body)
}

/// Finish the accounting for one request body.
pub fn settle(counter: &Counter, sent_this_attempt: &AtomicU64, committed_len: Option<u64>) {
    counter.sub_in_flight(sent_this_attempt.swap(0, Ordering::Relaxed));
    if let Some(len) = committed_len {
        counter.commit(len);
    }
}
