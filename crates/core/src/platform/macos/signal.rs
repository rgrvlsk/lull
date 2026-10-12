//! `Signaler` (`killpg`) and `Spawner` (login shell under `setsid`), contract §7 and
//! ADR-0005. The safety checks before a signal live in lifecycle; these only deliver.

use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::traits::{Signal, Signaler, SpawnRequest, Spawner};

/// Delivers signals to whole process groups with `killpg(2)`.
#[derive(Debug, Default)]
pub struct KillpgSignaler;

impl Signaler for KillpgSignaler {
    fn signal_group(&mut self, pgid: u32, signal: Signal) -> io::Result<()> {
        // killpg(0) hits the caller's own group and pgid 1 is launchd: never, whatever
        // the caller checked.
        let Ok(pgid) = libc::pid_t::try_from(pgid) else {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        };
        if pgid <= 1 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let signal = match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        // SAFETY: killpg takes plain integers and touches no memory.
        if unsafe { libc::killpg(pgid, signal) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// Variables kept from the daemon's environment; the login shell rebuilds the rest.
const LOGIN_ENV: [&str; 6] = ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG"];

/// Runs `$SHELL -l -c 'exec "$@"' lull-restart <argv…>` in a new session (ADR-0005).
#[derive(Debug, Default)]
pub struct LoginShellSpawner;

impl Spawner for LoginShellSpawner {
    fn spawn(&mut self, request: &SpawnRequest) -> io::Result<u32> {
        if request.argv.is_empty() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        if let Some(dir) = Path::new(&request.log_path).parent() {
            fs::create_dir_all(dir)?;
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&request.log_path)?;
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
        let mut cmd = Command::new(shell);
        cmd.args(["-l", "-c", r#"exec "$@""#, "lull-restart"])
            .args(&request.argv)
            .current_dir(&request.cwd)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        for key in LOGIN_ENV {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        cmd.envs(&request.env);
        // SAFETY: setsid is async-signal-safe and uses no memory of the parent.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        // Reap the session leader so it does not linger as a zombie member of its group.
        // ponytail: one parked thread per restart; restarts are rare and manual.
        std::thread::spawn(move || child.wait());
        Ok(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Spawns and stops its own `sleep` child; never touches any other process.
    #[test]
    fn spawns_in_new_session_with_login_env_and_stops_it() {
        let dir = std::env::temp_dir().join(format!("lull-spawn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("logs/restart-1.log");
        let request = SpawnRequest {
            argv: vec!["/bin/sh".into(), "-c".into(), "env; exec sleep 30".into()],
            cwd: dir.display().to_string(),
            env: crate::testing::env(&[("PORT", "4321")]),
            log_path: log.display().to_string(),
        };
        let pid = LoginShellSpawner.spawn(&request).unwrap();
        // SAFETY: getpgid only reads the process table.
        assert_eq!(
            unsafe { libc::getpgid(pid as libc::pid_t) },
            pid as libc::pid_t
        );

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut text = String::new();
        while !text.contains("PORT=4321") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            text = fs::read_to_string(&log).unwrap_or_default();
        }
        assert!(text.contains("PORT=4321"), "log: {text}");
        // cargo sets this for the test process; the restarted server must not inherit it.
        assert!(!text.contains("CARGO_MANIFEST_DIR="), "log: {text}");

        KillpgSignaler.signal_group(pid, Signal::Term).unwrap();
        // SAFETY: kill with signal 0 only probes for existence.
        while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            assert!(Instant::now() < deadline, "child survived SIGTERM");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(KillpgSignaler.signal_group(1, Signal::Term).is_err());
        assert!(KillpgSignaler.signal_group(0, Signal::Term).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
