//! Logging with redaction (PLAN.md 9.7): rotating files in the platform log
//! folder, every line passed through the redaction filter, SDK wire logs off.

use std::io::{self, Write};
use std::path::Path;

use r2dock_core::redact::redact_text;
use r2dock_core::settings::LogLevel;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload;

pub type ReloadHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

pub fn filter_for(level: LogLevel) -> EnvFilter {
    let ours = match level {
        LogLevel::Standard => "info",
        LogLevel::Detailed => "debug",
    };
    // SDK and HTTP internals stay at warn so request details are never logged.
    EnvFilter::new(format!(
        "warn,r2dock={ours},r2dock_lib={ours},r2dock_core={ours},r2dock_r2={ours},r2dock_transfer={ours},r2dock_secrets={ours},aws_smithy_runtime=warn,aws_sdk_s3=warn,hyper=warn"
    ))
}

/// Buffers one log event and writes it redacted.
pub struct RedactingWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.buf);
        let redacted = redact_text(&text);
        self.buf.clear();
        self.inner.write_all(redacted.as_bytes())?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

pub struct RedactingMakeWriter<M> {
    inner: M,
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer(),
            buf: Vec::new(),
        }
    }
}

/// Start logging to `dir`. Returns a handle to change the level later.
pub fn init(dir: &Path, level: LogLevel) -> Option<ReloadHandle> {
    let _ = std::fs::create_dir_all(dir);
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("r2dock")
        .filename_suffix("log")
        .max_log_files(5)
        .build(dir)
        .ok()?;
    let (filter, handle) = reload::Layer::new(filter_for(level));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(RedactingMakeWriter { inner: appender });
    let registry = tracing_subscriber::registry().with(filter).with(file_layer);
    #[cfg(debug_assertions)]
    let registry = registry.with(
        tracing_subscriber::fmt::layer().with_writer(RedactingMakeWriter { inner: io::stderr }),
    );
    registry.try_init().ok()?;
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn fake_secret_never_reaches_log_output() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(RedactingMakeWriter {
                    inner: capture.clone(),
                }),
        );
        let fake = "FAKESECRET0123456789abcdef";
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("authorization: AWS4-HMAC-SHA256 Credential=AKID, Signature={fake}");
            tracing::info!(secret = fake, "field");
            tracing::info!("https://x/y?X-Amz-Signature={fake}&a=1");
        });
        let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!out.contains(fake), "{out}");
        assert!(out.contains("[REDACTED]"));
    }
}
