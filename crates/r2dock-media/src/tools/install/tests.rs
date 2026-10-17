use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Serves assets from memory, in small chunks.
#[derive(Default)]
struct MemFetch {
    files: HashMap<String, Vec<u8>>,
    /// Cancel this token after the first chunk.
    cancel_after_first: Option<CancellationToken>,
    opened: AtomicBool,
}

struct MemBody {
    data: Vec<u8>,
    pos: usize,
    cancel: Option<CancellationToken>,
}

impl Body for MemBody {
    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, ToolsError> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        if self.pos > 0
            && let Some(c) = &self.cancel
        {
            c.cancel();
            tokio::task::yield_now().await;
        }
        let end = (self.pos + 4096).min(self.data.len());
        let chunk = self.data[self.pos..end].to_vec();
        self.pos = end;
        Ok(Some(chunk))
    }
}

impl Fetch for MemFetch {
    type Body = MemBody;
    async fn open(&self, url: &str) -> Result<MemBody, ToolsError> {
        self.opened.store(true, Ordering::Relaxed);
        let name = url.rsplit('/').next().unwrap_or_default();
        let data = self
            .files
            .get(name)
            .cloned()
            .ok_or_else(|| ToolsError::Download("404".into()))?;
        Ok(MemBody {
            data,
            pos: 0,
            cancel: self.cancel_after_first.clone(),
        })
    }
}

/// Bytes that do not compress, so downloads take several chunks.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn sha(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// A package with fake binaries, and a fetcher that serves it.
fn fixture() -> (Package, MemFetch, Vec<u8>, Vec<u8>) {
    let ffmpeg = noise(300_000, 1);
    let ffprobe = noise(120_000, 2);
    let mut fetch = MemFetch::default();
    let mut asset = |name: &str, bin: &[u8]| {
        let packed = zstd::encode_all(bin, 3).unwrap();
        let file = leak(format!("{name}-test.zst"));
        let a = Asset {
            file,
            download_sha256: leak(sha(&packed)),
            binary_sha256: leak(sha(bin)),
            download_size: packed.len() as u64,
        };
        fetch.files.insert(file.to_string(), packed);
        a
    };
    let pkg = Package {
        platform: "test-os-test-arch",
        ffmpeg: asset("ffmpeg", &ffmpeg),
        ffprobe: asset("ffprobe", &ffprobe),
    };
    (pkg, fetch, ffmpeg, ffprobe)
}

fn leftovers(root: &Path) -> Vec<String> {
    std::fs::read_dir(root)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

async fn install_ok(root: &Path, pkg: &Package, fetch: &MemFetch) -> Tools {
    let mut last = (0, 0);
    let tools = install(root, pkg, fetch, &CancellationToken::new(), |d, t| {
        last = (d, t)
    })
    .await
    .unwrap();
    assert_eq!(last, (pkg.total_download(), pkg.total_download()));
    tools
}

#[tokio::test]
async fn installs_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, fetch, ffmpeg, ffprobe) = fixture();
    let tools = install_ok(dir.path(), &pkg, &fetch).await;
    assert_eq!(std::fs::read(&tools.ffmpeg).unwrap(), ffmpeg);
    assert_eq!(std::fs::read(&tools.ffprobe).unwrap(), ffprobe);
    assert_eq!(tools.version, manifest::VERSION);
    assert_eq!(tools.source, ToolsSource::Bundled);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&tools.ffmpeg)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert_eq!(check_installed(dir.path(), Some(&pkg)).unwrap(), tools);
    assert_eq!(
        leftovers(dir.path()),
        vec![format!("ffmpeg-{}", manifest::VERSION)]
    );
}

#[tokio::test]
async fn refuses_a_tampered_download_before_unpacking() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, mut fetch, _, _) = fixture();
    let packed = fetch.files.get_mut(pkg.ffprobe.file).unwrap();
    packed[10] ^= 0xff;
    let err = install(
        dir.path(),
        &pkg,
        &fetch,
        &CancellationToken::new(),
        |_, _| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ToolsError::ChecksumMismatch), "{err:?}");
    assert!(
        leftovers(dir.path()).is_empty(),
        "{:?}",
        leftovers(dir.path())
    );
    assert!(matches!(
        check_installed(dir.path(), Some(&pkg)),
        Err(ToolsError::NotInstalled)
    ));
}

#[tokio::test]
async fn refuses_a_wrong_binary() {
    let dir = tempfile::tempdir().unwrap();
    let (mut pkg, fetch, _, _) = fixture();
    pkg.ffmpeg.binary_sha256 = "00";
    let err = install(
        dir.path(),
        &pkg,
        &fetch,
        &CancellationToken::new(),
        |_, _| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ToolsError::ChecksumMismatch));
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn refuses_an_oversized_download() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, mut fetch, _, _) = fixture();
    fetch
        .files
        .get_mut(pkg.ffmpeg.file)
        .unwrap()
        .extend(std::iter::repeat_n(0u8, 2 * 1024 * 1024));
    let err = install(
        dir.path(),
        &pkg,
        &fetch,
        &CancellationToken::new(),
        |_, _| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ToolsError::ChecksumMismatch));
}

#[tokio::test]
async fn detects_a_replaced_binary() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, fetch, ffmpeg, _) = fixture();
    let tools = install_ok(dir.path(), &pkg, &fetch).await;
    // Same size, different bytes, new modification time.
    let mut evil = ffmpeg.clone();
    evil[0] ^= 0xff;
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&tools.ffmpeg, &evil).unwrap();
    assert!(matches!(
        check_installed(dir.path(), Some(&pkg)),
        Err(ToolsError::Tampered)
    ));
    // Reinstalling fixes it.
    install_ok(dir.path(), &pkg, &fetch).await;
    check_installed(dir.path(), Some(&pkg)).unwrap();
}

#[tokio::test]
async fn pinned_sums_win_over_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, fetch, _, _) = fixture();
    install_ok(dir.path(), &pkg, &fetch).await;
    let mut newer = pkg;
    newer.ffprobe.binary_sha256 = "ab";
    assert!(matches!(
        check_installed(dir.path(), Some(&newer)),
        Err(ToolsError::Tampered)
    ));
    // Without a pinned package (system check), the record alone decides.
    check_installed(dir.path(), None).unwrap();
}

#[tokio::test]
async fn edited_records_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, fetch, _, _) = fixture();
    install_ok(dir.path(), &pkg, &fetch).await;
    let record = install_dir(dir.path()).join(RECORD);
    let text = std::fs::read_to_string(&record).unwrap();
    let name = exe("ffmpeg");
    std::fs::write(
        &record,
        text.replace(&format!("\"{name}\""), "\"../../evil\""),
    )
    .unwrap();
    assert!(matches!(
        check_installed(dir.path(), None),
        Err(ToolsError::Tampered)
    ));
    std::fs::write(&record, "not json").unwrap();
    assert!(matches!(
        check_installed(dir.path(), None),
        Err(ToolsError::Tampered)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_binaries_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, fetch, _, _) = fixture();
    let tools = install_ok(dir.path(), &pkg, &fetch).await;
    std::fs::remove_file(&tools.ffmpeg).unwrap();
    std::os::unix::fs::symlink("/bin/sh", &tools.ffmpeg).unwrap();
    assert!(matches!(
        check_installed(dir.path(), Some(&pkg)),
        Err(ToolsError::Tampered)
    ));
}

#[tokio::test]
async fn cancel_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, mut fetch, _, _) = fixture();
    let token = CancellationToken::new();
    fetch.cancel_after_first = Some(token.clone());
    let err = install(dir.path(), &pkg, &fetch, &token, |_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, ToolsError::Cancelled), "{err:?}");
    assert!(fetch.opened.load(Ordering::Relaxed));
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn remove_deletes_everything() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, fetch, _, _) = fixture();
    install_ok(dir.path(), &pkg, &fetch).await;
    std::fs::create_dir(dir.path().join(".staging-old")).unwrap();
    std::fs::write(dir.path().join("keep.txt"), b"x").unwrap();
    remove(dir.path()).unwrap();
    assert_eq!(leftovers(dir.path()), vec!["keep.txt".to_string()]);
    remove(&dir.path().join("missing")).unwrap();
}

#[test]
fn only_github_hosts() {
    let ok = |u: &str| allowed(&reqwest::Url::parse(u).unwrap());
    assert!(ok(
        "https://github.com/VijaysinghPuwar/R2Dock/releases/download/x/y"
    ));
    assert!(ok("https://release-assets.githubusercontent.com/x"));
    assert!(ok("https://objects.githubusercontent.com/x"));
    assert!(!ok("http://github.com/x"));
    assert!(!ok("https://github.com.evil.example/x"));
    assert!(!ok("https://example.com/x"));
}

#[test]
fn manifest_is_consistent() {
    for p in manifest::PACKAGES {
        for a in [&p.ffmpeg, &p.ffprobe] {
            assert_eq!(a.download_sha256.len(), 64, "{}", a.file);
            assert_eq!(a.binary_sha256.len(), 64, "{}", a.file);
            assert!(a.file.ends_with(".zst"));
            assert!(a.download_size > 0);
            assert!(
                manifest::url(a)
                    .starts_with("https://github.com/VijaysinghPuwar/R2Dock/releases/download/")
            );
        }
    }
    let platforms: Vec<_> = manifest::PACKAGES.iter().map(|p| p.platform).collect();
    let mut unique = platforms.clone();
    unique.dedup();
    assert_eq!(platforms, unique);
}
