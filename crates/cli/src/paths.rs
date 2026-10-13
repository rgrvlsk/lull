//! Where lull keeps its files (contract §3, §6, §7, §8).

use std::path::PathBuf;

pub struct Paths {
    /// `~/Library/Application Support/lull`: socket, lock, state, config.
    pub dir: PathBuf,
    /// `~/Library/Logs`: `lull.log` and `lull/restart-<id>.log`.
    pub logs: PathBuf,
}

impl Paths {
    /// `LULL_DIR` moves everything under one directory (tests, e2e); unset means the
    /// contract's locations.
    pub fn from_env() -> Paths {
        if let Some(dir) = std::env::var_os("LULL_DIR") {
            let dir = PathBuf::from(dir);
            return Paths {
                logs: dir.join("logs"),
                dir,
            };
        }
        let home = PathBuf::from(home());
        Paths {
            dir: home.join("Library/Application Support/lull"),
            logs: home.join("Library/Logs"),
        }
    }
    pub fn socket(&self) -> PathBuf {
        self.dir.join("daemon.sock")
    }
    pub fn lock(&self) -> PathBuf {
        self.dir.join("daemon.lock")
    }
    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    pub fn config(&self) -> PathBuf {
        self.dir.join("config.toml")
    }
    pub fn log_file(&self) -> PathBuf {
        self.logs.join("lull.log")
    }
    pub fn restart_logs(&self) -> PathBuf {
        self.logs.join("lull")
    }
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}
