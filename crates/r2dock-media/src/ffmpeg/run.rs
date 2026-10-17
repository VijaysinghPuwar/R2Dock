//! Run FFmpeg as a separate process and watch it.
//!
//! - Started with an argument array, never a shell; stdin is closed.
//! - stdout (progress) and stderr (errors) are drained continuously, so a
//!   full pipe can never block FFmpeg. The last lines of stderr are kept.
//! - Cancel kills the process; nothing is kept from a cancelled encode.
//! - A watchdog kills FFmpeg if it prints nothing for too long.
//! - FFmpeg dies with the app: on Windows it runs in a Job Object that is
//!   killed when R2Dock exits; on macOS and Linux it runs in its own process
//!   group and exits on the next progress write after R2Dock is gone (the
//!   pipe is closed). In every case the child is killed when dropped.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use process_wrap::tokio::{CommandWrap, KillOnDrop};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::progress::{Progress, ProgressParser};

#[derive(Debug, Error)]
pub enum RunError {
    #[error("could not start FFmpeg")]
    Spawn(#[source] std::io::Error),
    #[error("cancelled")]
    Cancelled,
    #[error("FFmpeg stopped responding")]
    Stalled,
    #[error("FFmpeg failed")]
    Failed {
        code: Option<i32>,
        /// Last line FFmpeg printed, if any (may contain paths; sanitize).
        last_line: Option<String>,
    },
    #[error("lost contact with FFmpeg")]
    Io(#[source] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    /// Kill FFmpeg after this long without any output.
    pub watchdog: Duration,
    /// How many stderr lines to keep.
    pub keep_lines: usize,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            watchdog: Duration::from_secs(120),
            keep_lines: 50,
        }
    }
}

enum Line {
    Out(String),
    Err(String),
}

fn forward<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    tx: mpsc::Sender<Line>,
    make: fn(String) -> Line,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf)
                        .trim_end_matches(['\n', '\r'])
                        .to_string();
                    if tx.send(make(line)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

fn command(bin: &Path, args: &[OsString]) -> CommandWrap {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000 | 0x0000_4000);
    let mut wrap = CommandWrap::from(cmd);
    #[cfg(unix)]
    wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    wrap.wrap(process_wrap::tokio::JobObject);
    wrap.wrap(KillOnDrop);
    wrap
}

/// Run FFmpeg with `args`, calling `on_progress` for each progress report.
/// Returns the last stderr lines on success.
pub async fn run(
    bin: &Path,
    args: &[OsString],
    cancel: &CancellationToken,
    opts: RunOptions,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Result<Vec<String>, RunError> {
    let mut child = command(bin, args).spawn().map_err(RunError::Spawn)?;
    let (tx, mut rx) = mpsc::channel::<Line>(256);
    if let Some(out) = child.stdout().take() {
        forward(out, tx.clone(), Line::Out);
    }
    if let Some(err) = child.stderr().take() {
        forward(err, tx, Line::Err);
    }
    let mut parser = ProgressParser::default();
    let mut tail: VecDeque<String> = VecDeque::with_capacity(opts.keep_lines);
    // Read until both pipes close, watching for cancel and silence.
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                let _ = Box::into_pin(child.kill()).await;
                return Err(RunError::Cancelled);
            }
            line = tokio::time::timeout(opts.watchdog, rx.recv()) => match line {
                Err(_) => {
                    let _ = Box::into_pin(child.kill()).await;
                    return Err(RunError::Stalled);
                }
                Ok(None) => break,
                Ok(Some(Line::Out(l))) => {
                    if let Some(p) = parser.line(&l) {
                        on_progress(p);
                    }
                }
                Ok(Some(Line::Err(l))) => {
                    if !l.trim().is_empty() {
                        if tail.len() == opts.keep_lines {
                            tail.pop_front();
                        }
                        tail.push_back(l);
                    }
                }
            },
        }
    }
    let status = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            let _ = Box::into_pin(child.kill()).await;
            return Err(RunError::Cancelled);
        }
        status = tokio::time::timeout(opts.watchdog, child.wait()) => match status {
            Err(_) => {
                let _ = Box::into_pin(child.kill()).await;
                return Err(RunError::Stalled);
            }
            Ok(s) => s.map_err(RunError::Io)?,
        },
    };
    if status.success() {
        Ok(tail.into_iter().collect())
    } else {
        Err(RunError::Failed {
            code: status.code(),
            last_line: tail.back().cloned(),
        })
    }
}

/// Replace local paths in an FFmpeg message with just the file names, so
/// messages never show the user's folders.
pub fn sanitize(line: &str, paths: &[&Path]) -> String {
    let mut out = line.to_string();
    for p in paths {
        let full = p.display().to_string();
        if full.is_empty() {
            continue;
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out = out.replace(&full, &name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_folders() {
        let p = Path::new("/Users/someone/Music/Book One/part 1.mp3");
        let line = "file:/Users/someone/Music/Book One/part 1.mp3: Invalid data found";
        assert_eq!(sanitize(line, &[p]), "file:part 1.mp3: Invalid data found");
        assert_eq!(sanitize("no paths here", &[p]), "no paths here");
    }
}
