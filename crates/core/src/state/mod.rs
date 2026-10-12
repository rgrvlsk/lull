//! `state.json` persistence (contract §4.7). The schema belongs to lifecycle; this store
//! only moves bytes atomically.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::traits::StateStore;

/// `StateStore` on a file: written to `<path>.tmp`, synced, then renamed over `path`.
#[derive(Debug, Clone)]
pub struct FileStateStore {
    path: PathBuf,
}

impl FileStateStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(suffix);
        PathBuf::from(name)
    }
}

impl StateStore for FileStateStore {
    fn load(&mut self) -> io::Result<Option<serde_json::Value>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }

    fn save(&mut self, state: &serde_json::Value) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.sibling(".tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(state)?)?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)
    }

    fn quarantine(&mut self) -> io::Result<String> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let dest = self.sibling(&format!(".corrupt-{ts}"));
        fs::rename(&self.path, &dest)?;
        Ok(dest.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lull-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn saves_atomically_and_loads_back() {
        let dir = temp_dir("roundtrip");
        let mut store = FileStateStore::new(dir.join("nested/state.json"));
        assert_eq!(store.load().unwrap(), None);
        let value = serde_json::json!({"version": 1, "servers": {}});
        store.save(&value).unwrap();
        assert_eq!(store.load().unwrap(), Some(value));
        assert!(!dir.join("nested/state.json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_invalid_data_and_quarantined() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, "{not json").unwrap();
        let mut store = FileStateStore::new(path.clone());
        assert_eq!(store.load().unwrap_err().kind(), io::ErrorKind::InvalidData);
        let moved = store.quarantine().unwrap();
        assert!(moved.contains("state.json.corrupt-"));
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&moved).unwrap(), "{not json");
        fs::remove_dir_all(dir).unwrap();
    }
}
