//! Download, verify, unpack, and check the pinned FFmpeg package.
//!
//! Layout under the tools folder (in the app data folder):
//!
//! ```text
//! ffmpeg-9.0.2/ffmpeg[.exe]
//! ffmpeg-9.0.2/ffprobe[.exe]
//! ffmpeg-9.0.2/installed.json   sizes, modification times, and SHA-256
//! .staging-*/                   work in progress, deleted on any failure
//! ```
//!
//! Every download is hashed while it is written and checked against the
//! pinned sum before it is unpacked. The unpacked binary is hashed again.
//! The finished folder is moved into place in one rename. Before each use,
//! a binary whose size or modification time changed is hashed again; a
//! mismatch means it was replaced, and it is refused.

use std::future::Future;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::manifest::{self, Asset, Package};
use super::{Tools, ToolsError, ToolsSource};

/// A response body read in chunks.
pub trait Body: Send {
    fn chunk(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, ToolsError>> + Send;
}

/// Opens a URL for download.
pub trait Fetch: Sync {
    type Body: Body;
    fn open(&self, url: &str) -> impl Future<Output = Result<Self::Body, ToolsError>> + Send;
}

/// Downloads over HTTPS. Redirects are followed only to GitHub's own hosts.
pub struct HttpFetch {
    client: reqwest::Client,
}

const ALLOWED_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

fn allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))
}

impl HttpFetch {
    pub fn new(user_agent: &str) -> Result<Self, ToolsError> {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !allowed(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .redirect(policy)
            .user_agent(user_agent)
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(20))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| ToolsError::Download(e.to_string()))?;
        Ok(Self { client })
    }
}

pub struct HttpBody(reqwest::Response);

impl Body for HttpBody {
    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, ToolsError> {
        self.0
            .chunk()
            .await
            .map(|c| c.map(|b| b.to_vec()))
            .map_err(|_| ToolsError::Download("the connection was interrupted".into()))
    }
}

impl Fetch for HttpFetch {
    type Body = HttpBody;
    async fn open(&self, url: &str) -> Result<HttpBody, ToolsError> {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| ToolsError::Download("bad URL".into()))?;
        if !allowed(&parsed) {
            return Err(ToolsError::Download("download host not allowed".into()));
        }
        let response = self
            .client
            .get(parsed)
            .send()
            .await
            .map_err(|_| ToolsError::Download("could not connect".into()))?;
        let status = response.status();
        if !status.is_success() || !allowed(response.url()) {
            return Err(ToolsError::Download(format!("server answered {status}")));
        }
        Ok(HttpBody(response))
    }
}

/// What was installed, kept next to the binaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub version: String,
    pub platform: String,
    pub ffmpeg: BinaryRecord,
    pub ffprobe: BinaryRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryRecord {
    pub file: String,
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub modified_ns: u128,
    pub sha256: String,
}

const RECORD: &str = "installed.json";

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Folder of the installed package.
pub fn install_dir(root: &Path) -> PathBuf {
    root.join(format!("ffmpeg-{}", manifest::VERSION))
}

/// Hashes everything written through it.
struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Download `asset` to `dest`, checking its pinned sum. Returns bytes read.
async fn download<F: Fetch>(
    fetch: &F,
    asset: &Asset,
    dest: &Path,
    cancel: &CancellationToken,
    on_bytes: &mut (dyn FnMut(u64) + Send),
) -> Result<(), ToolsError> {
    let mut body = fetch.open(&manifest::url(asset)).await?;
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    // A little slack over the pinned size; anything bigger is not our file.
    let limit = asset.download_size.saturating_add(1024 * 1024);
    loop {
        let chunk = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ToolsError::Cancelled),
            c = body.chunk() => c?,
        };
        let Some(chunk) = chunk else { break };
        total += chunk.len() as u64;
        if total > limit {
            return Err(ToolsError::ChecksumMismatch);
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        on_bytes(chunk.len() as u64);
    }
    file.flush().await?;
    drop(file);
    if hex::encode(hasher.finalize()) != asset.download_sha256 {
        let _ = std::fs::remove_file(dest);
        return Err(ToolsError::ChecksumMismatch);
    }
    Ok(())
}

/// Unpack a verified `.zst` file and check the binary's sum. Blocking.
fn unpack(compressed: &Path, dest: &Path, binary_sha256: &str) -> Result<BinaryRecord, ToolsError> {
    let input = std::fs::File::open(compressed)?;
    let mut decoder = zstd::stream::read::Decoder::new(input)?;
    let mut out = HashingWriter {
        inner: std::fs::File::create(dest)?,
        hasher: Sha256::new(),
    };
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = decoder
            .read(&mut buf)
            .map_err(|_| ToolsError::ChecksumMismatch)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    let sha = hex::encode(out.hasher.finalize());
    drop(out.inner);
    if sha != binary_sha256 {
        let _ = std::fs::remove_file(dest);
        return Err(ToolsError::ChecksumMismatch);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    record_of(dest, sha)
}

fn record_of(path: &Path, sha256: String) -> Result<BinaryRecord, ToolsError> {
    let meta = std::fs::metadata(path)?;
    Ok(BinaryRecord {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: meta.len(),
        modified_ns: modified_ns(&meta),
        sha256,
    })
}

fn modified_ns(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

fn sha256_file(path: &Path) -> Result<String, ToolsError> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut f, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// Download and install `pkg` into `root`. `on_progress` receives
/// (downloaded bytes, total bytes).
pub async fn install<F: Fetch>(
    root: &Path,
    pkg: &Package,
    fetch: &F,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(u64, u64) + Send,
) -> Result<Tools, ToolsError> {
    std::fs::create_dir_all(root)?;
    // Deleted automatically if anything below fails or is cancelled.
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(root)?;
    let total = pkg.total_download();
    let mut done = 0u64;
    let mut on_bytes = |n: u64| {
        done += n;
        on_progress(done, total);
    };
    let mut records = Vec::new();
    for (name, asset) in [("ffmpeg", &pkg.ffmpeg), ("ffprobe", &pkg.ffprobe)] {
        let compressed = staging.path().join(asset.file);
        download(fetch, asset, &compressed, cancel, &mut on_bytes).await?;
        let binary = staging.path().join(exe(name));
        let expected = asset.binary_sha256.to_string();
        let record = tokio::task::spawn_blocking(move || {
            let r = unpack(&compressed, &binary, &expected);
            let _ = std::fs::remove_file(&compressed);
            r
        })
        .await
        .map_err(|_| ToolsError::Io(io::Error::other("unpack task stopped")))??;
        records.push(record);
    }
    let ffprobe = records.pop().ok_or(ToolsError::NotInstalled)?;
    let ffmpeg = records.pop().ok_or(ToolsError::NotInstalled)?;
    let installed = Installed {
        version: manifest::VERSION.to_string(),
        platform: pkg.platform.to_string(),
        ffmpeg,
        ffprobe,
    };
    let json = serde_json::to_vec_pretty(&installed).map_err(io::Error::other)?;
    std::fs::write(staging.path().join(RECORD), json)?;

    let dest = install_dir(root);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    let staged = staging.keep();
    std::fs::rename(&staged, &dest)?;
    check_installed(root, Some(pkg))
}

/// The installed tools, after checking nothing was replaced. Blocking when
/// a binary has to be hashed again.
pub fn check_installed(root: &Path, pkg: Option<&Package>) -> Result<Tools, ToolsError> {
    let dir = install_dir(root);
    let text = std::fs::read(dir.join(RECORD)).map_err(|_| ToolsError::NotInstalled)?;
    let installed: Installed = serde_json::from_slice(&text).map_err(|_| ToolsError::Tampered)?;
    let pinned = pkg.map(|p| (p.ffmpeg.binary_sha256, p.ffprobe.binary_sha256));
    for (record, pinned_sha) in [
        (&installed.ffmpeg, pinned.map(|p| p.0)),
        (&installed.ffprobe, pinned.map(|p| p.1)),
    ] {
        // The record must name a plain file in this folder.
        if record.file.contains(['/', '\\']) || record.file.starts_with('.') {
            return Err(ToolsError::Tampered);
        }
        let path = dir.join(&record.file);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| ToolsError::NotInstalled)?;
        if !meta.is_file() {
            return Err(ToolsError::Tampered);
        }
        let unchanged = meta.len() == record.size && modified_ns(&meta) == record.modified_ns;
        if !unchanged && sha256_file(&path)? != record.sha256 {
            return Err(ToolsError::Tampered);
        }
        if pinned_sha.is_some_and(|s| s != record.sha256) {
            return Err(ToolsError::Tampered);
        }
    }
    Ok(Tools {
        ffmpeg: dir.join(&installed.ffmpeg.file),
        ffprobe: dir.join(&installed.ffprobe.file),
        version: installed.version,
        source: ToolsSource::Bundled,
    })
}

/// Remove the installed tools and any unfinished install.
pub fn remove(root: &Path) -> Result<(), ToolsError> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("ffmpeg-") || name.starts_with(".staging-") {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
