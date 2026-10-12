//! `Clock` over `CLOCK_UPTIME_RAW` and `kern.bootsessionuuid` (contract §5, "Clock").

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::traits::{Clock, Instant};

/// Awake-time clock: `CLOCK_UPTIME_RAW` stops while the Mac sleeps; the boot session uuid
/// is read once per process so every instant carries the same session string.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacClock;

unsafe extern "C" {
    fn clock_gettime_nsec_np(clock_id: libc::clockid_t) -> u64;
}

static BOOT_SESSION: OnceLock<String> = OnceLock::new();

impl MacClock {
    pub fn new() -> Self {
        let _ = Self::boot_session();
        Self
    }

    /// `kern.bootsessionuuid`, read once. Failure uses a process-local session so
    /// persisted instants cannot match a later daemon start.
    pub fn boot_session() -> &'static str {
        BOOT_SESSION.get_or_init(read_boot_session)
    }
}

fn read_boot_session() -> String {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    // SAFETY: the name is a NUL-terminated literal; `buf`/`len` describe a valid buffer
    // and the kernel writes at most `len` bytes, updating `len`.
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len > buf.len() || len <= 1 || buf[0] == 0 {
        return format!("unavailable-{}-{:?}", std::process::id(), SystemTime::now());
    }
    let end = buf[..len].iter().position(|&b| b == 0).unwrap_or(len);
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl Clock for MacClock {
    fn now(&self) -> Instant {
        // SAFETY: CLOCK_UPTIME_RAW is supported on the deployment target (macOS 14+).
        let uptime_ms = unsafe { clock_gettime_nsec_np(libc::CLOCK_UPTIME_RAW) } / 1_000_000;
        Instant {
            uptime_ms,
            boot_session: Self::boot_session().to_owned(),
        }
    }

    fn wall_now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_advances_within_one_boot_session() {
        let clock = MacClock;
        let a = clock.now();
        let b = clock.now();
        assert_eq!(a.boot_session, b.boot_session);
        assert!(!a.boot_session.is_empty());
        assert!(b.ms_since(&a).is_some());
        assert!(clock.wall_now_ms() > 1_700_000_000_000);
    }
}
