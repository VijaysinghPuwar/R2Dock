//! Atomic JSON files for config, settings, and the transfer journal.
//!
//! Writes go to a temp file in the same folder, are flushed to disk, then
//! renamed over the target. A file that fails to parse is moved aside as a
//! backup instead of being silently overwritten.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("could not access local data: {0:?}")]
    Io(io::ErrorKind),
    #[error("could not encode local data")]
    Encode,
}

impl From<io::Error> for StorageError {
    fn from(err: io::Error) -> Self {
        Self::Io(err.kind())
    }
}

/// Outcome of loading a JSON file.
#[derive(Debug)]
pub enum Loaded<T> {
    Found(T),
    Missing,
    /// The file could not be parsed. It was renamed to the given backup path.
    Corrupt {
        backup: PathBuf,
    },
}

/// Write `value` to `path` atomically.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    let dir = path
        .parent()
        .ok_or(StorageError::Io(io::ErrorKind::InvalidInput))?;
    fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| StorageError::Encode)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".r2dock-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(&bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| StorageError::Io(e.error.kind()))?;
    Ok(())
}

/// Read a JSON file. Missing files are `Missing`; unreadable ones are backed up.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Loaded<T>, StorageError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(err) => return Err(err.into()),
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Loaded::Found(value)),
        Err(_) => {
            let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
            let mut backup = path.as_os_str().to_owned();
            backup.push(format!(".corrupt-{stamp}"));
            let backup = PathBuf::from(backup);
            fs::rename(path, &backup)?;
            tracing::warn!("local data file could not be read and was backed up");
            Ok(Loaded::Corrupt { backup })
        }
    }
}

/// Read a file or fall back to the default value.
pub fn read_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T, StorageError> {
    match read_json(path)? {
        Loaded::Found(v) => Ok(v),
        Loaded::Missing | Loaded::Corrupt { .. } => Ok(T::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Doc {
        version: u32,
        name: String,
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("doc.json");
        let doc = Doc {
            version: 1,
            name: "a".into(),
        };
        write_json_atomic(&path, &doc).unwrap();
        match read_json::<Doc>(&path).unwrap() {
            Loaded::Found(d) => assert_eq!(d, doc),
            other => panic!("unexpected {other:?}"),
        }
        // No temp files left behind.
        let leftovers = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn missing_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_json::<Doc>(&dir.path().join("x.json")).unwrap(),
            Loaded::Missing
        ));
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        fs::write(&path, b"{not json").unwrap();
        let loaded = read_json::<Doc>(&path).unwrap();
        let Loaded::Corrupt { backup } = loaded else {
            panic!("expected corrupt")
        };
        assert!(backup.exists());
        assert!(!path.exists());
        assert_eq!(read_json_or_default::<Doc>(&path).unwrap(), Doc::default());
    }
}
