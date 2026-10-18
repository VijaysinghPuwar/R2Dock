//! Opening local files for upload safely (PLAN.md 9.6).
//!
//! A file is opened without following a final symlink and its identity is
//! remembered. Every later read (one per part, and on retries) re-opens the
//! path and checks it is still the same file, so a file swapped after the
//! scan is never uploaded by mistake.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use md5::{Digest, Md5};
use same_file::Handle;

use crate::retry::JobError;

/// Size and modification time, used to detect changes during an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fingerprint {
    pub len: u64,
    /// Nanoseconds since the Unix epoch (0 when unknown). A `u64` covers
    /// dates up to the year 2554 and, unlike `u128`, can be read back from
    /// the journal, whose entries are internally tagged serde enums.
    pub mtime_ns: u64,
}

impl Fingerprint {
    fn of(meta: &std::fs::Metadata) -> Self {
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        Self {
            len: meta.len(),
            mtime_ns,
        }
    }
}

fn open_nofollow(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open a link itself instead of its target.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    opts.open(path)
}

/// Identity of a file recorded when it was scanned (device and inode on
/// Unix). Used to refuse a file that was swapped after the scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
}

impl FileIdentity {
    /// Identity from metadata, where the platform provides one.
    pub fn of(meta: &std::fs::Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(Self {
                dev: meta.dev(),
                ino: meta.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            None
        }
    }
}

/// What a file R2Dock wrote looked like when it was finished: size,
/// modification time, and (where the platform provides one) its identity.
/// A move within the same folder keeps all three, so a file that matches
/// is the one R2Dock wrote and not an unrelated file of the same size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WrittenFile {
    pub len: u64,
    pub mtime_ns: u64,
    #[serde(default)]
    pub identity: Option<FileIdentity>,
}

impl WrittenFile {
    pub fn of(meta: &std::fs::Metadata) -> Self {
        let fp = Fingerprint::of(meta);
        Self {
            len: fp.len,
            mtime_ns: fp.mtime_ns,
            identity: FileIdentity::of(meta),
        }
    }

    /// Whether the file at `path` (not following a symbolic link) is this one.
    pub fn matches(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && Self::of(&m) == *self)
    }
}

/// SHA-256 (hex) of a file R2Dock wrote, read without following a
/// symbolic link, with what the file looked like. Fails if the file
/// changed while it was read. Blocking.
pub fn digest_written(path: &Path) -> std::io::Result<(WrittenFile, String)> {
    use sha2::{Digest as _, Sha256};
    let mut file = open_nofollow(path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let written = WrittenFile::of(&before);
    if WrittenFile::of(&file.metadata()?) != written {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    Ok((written, hex::encode(hasher.finalize())))
}

/// A local file checked for upload.
#[derive(Debug)]
pub struct SourceFile {
    path: PathBuf,
    identity: Handle,
    pub fingerprint: Fingerprint,
}

impl SourceFile {
    /// Open and check: must be a regular file, not a symbolic link, and (when
    /// known) the same file that was scanned.
    pub fn open(path: &Path, expected: Option<FileIdentity>) -> Result<Self, JobError> {
        let file = open_nofollow(path).map_err(|e| map_open_error(&e))?;
        let meta = file.metadata().map_err(|e| JobError::io(&e, "read file"))?;
        if let (Some(want), Some(have)) = (expected, FileIdentity::of(&meta))
            && want != have
        {
            return Err(JobError::file_changed());
        }
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(JobError::fatal(r2dock_core::AppError::invalid_input(
                "Only regular files can be uploaded. Shortcuts and special files are skipped.",
            )));
        }
        let identity = Handle::from_file(file).map_err(|e| JobError::io(&e, "read file"))?;
        Ok(Self {
            path: path.to_path_buf(),
            identity,
            fingerprint: Fingerprint::of(&meta),
        })
    }

    /// Re-open the same file. Fails if the path now points at a different file.
    pub fn reopen(&self) -> Result<File, JobError> {
        let file = open_nofollow(&self.path).map_err(|e| map_open_error(&e))?;
        let clone = file
            .try_clone()
            .map_err(|e| JobError::io(&e, "read file"))?;
        let handle = Handle::from_file(clone).map_err(|e| JobError::io(&e, "read file"))?;
        if handle != self.identity {
            return Err(JobError::file_changed());
        }
        Ok(file)
    }

    /// Current fingerprint of the file at the path.
    pub fn current_fingerprint(&self) -> Result<Fingerprint, JobError> {
        let file = self.reopen()?;
        let meta = file.metadata().map_err(|e| JobError::io(&e, "read file"))?;
        Ok(Fingerprint::of(&meta))
    }

    /// Fail if the file changed since it was opened.
    pub fn ensure_unchanged(&self) -> Result<(), JobError> {
        if self.current_fingerprint()? != self.fingerprint {
            return Err(JobError::file_changed());
        }
        Ok(())
    }

    /// MD5 of a byte range, read with a fixed 1 MiB buffer. Blocking.
    pub fn md5_range(&self, offset: u64, len: u64) -> Result<[u8; 16], JobError> {
        use std::io::{Seek, SeekFrom};
        let mut file = self.reopen()?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| JobError::io(&e, "read file"))?;
        let mut reader = file.take(len);
        let mut hasher = Md5::new();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut read_total = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| JobError::io(&e, "read file"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            read_total += n as u64;
        }
        if read_total != len {
            return Err(JobError::file_changed());
        }
        Ok(hasher.finalize().into())
    }
}

fn map_open_error(err: &std::io::Error) -> JobError {
    // O_NOFOLLOW on a symlink fails with ELOOP.
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::ELOOP) {
        return JobError::fatal(r2dock_core::AppError::invalid_input(
            "This is a shortcut (symbolic link), so it was skipped.",
        ));
    }
    JobError::io(err, "read file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_regular_file_and_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"hello world").unwrap();
        let s = SourceFile::open(&p, None).unwrap();
        assert_eq!(s.fingerprint.len, 11);
        let md5 = s.md5_range(0, 5).unwrap();
        assert_eq!(
            hex::encode(md5),
            hex::encode(r2dock_core::etag::md5_bytes(b"hello"))
        );
        s.ensure_unchanged().unwrap();
    }

    #[test]
    fn detects_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"one").unwrap();
        let s = SourceFile::open(&p, None).unwrap();
        std::fs::remove_file(&p).unwrap();
        std::fs::write(dir.path().join("b.txt"), b"other").unwrap();
        std::fs::rename(dir.path().join("b.txt"), &p).unwrap();
        assert_eq!(
            s.reopen().unwrap_err().app.code,
            r2dock_core::ErrorCode::FileChanged
        );
    }

    #[test]
    fn detects_modification() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"one").unwrap();
        let s = SourceFile::open(&p, None).unwrap();
        std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(b"more").unwrap();
        }
        assert!(s.ensure_unchanged().is_err());
        // Reading past the new end of a shrunk file also fails.
        assert!(s.md5_range(0, 100).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t.txt");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("l.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(SourceFile::open(&link, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_different_file_than_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let id_a = FileIdentity::of(&std::fs::symlink_metadata(&a).unwrap());
        assert!(SourceFile::open(&a, id_a).is_ok());
        assert_eq!(
            SourceFile::open(&b, id_a).unwrap_err().app.code,
            r2dock_core::ErrorCode::FileChanged
        );
    }

    #[test]
    fn refuses_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SourceFile::open(dir.path(), None).is_err());
    }
}
