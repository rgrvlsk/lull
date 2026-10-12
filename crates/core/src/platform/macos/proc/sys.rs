//! Thin, bounds-checked wrappers over libproc and sysctl. All `unsafe` of the process
//! source lives here.

use std::collections::BTreeMap;
use std::ffi::{CStr, c_int};
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::ptr::null_mut;
use std::sync::OnceLock;

/// `e_tdev` of a process whose session has no controlling terminal.
pub(super) const NODEV: u32 = u32::MAX;
const PROC_PGRP_ONLY: u32 = 2;
const PROC_UID_ONLY: u32 = 4;
const PROC_PIDFDSOCKETINFO: c_int = 3;

/// `proc_pidinfo` for a flavor whose result is the plain C struct `T`.
fn pidinfo<T: Copy>(pid: u32, flavor: c_int) -> io::Result<T> {
    let mut out = MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as c_int;
    // SAFETY: the kernel writes at most `size` bytes into `out`.
    let n = unsafe { libc::proc_pidinfo(pid as c_int, flavor, 0, out.as_mut_ptr().cast(), size) };
    match n {
        // SAFETY: `T` is a libc C struct, valid for any bit pattern (zeroed or filled).
        n if n == size => Ok(unsafe { out.assume_init() }),
        n if n <= 0 => Err(io::Error::last_os_error()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short proc_pidinfo",
        )),
    }
}

pub(super) fn bsdinfo(pid: u32) -> io::Result<libc::proc_bsdinfo> {
    pidinfo(pid, libc::PROC_PIDTBSDINFO)
}

pub(super) fn shortinfo(pid: u32) -> io::Result<libc::proc_bsdshortinfo> {
    pidinfo(pid, libc::PROC_PIDT_SHORTBSDINFO)
}

pub(super) fn taskinfo(pid: u32) -> io::Result<libc::proc_taskinfo> {
    pidinfo(pid, libc::PROC_PIDTASKINFO)
}

fn c_string(bytes: &[u8]) -> io::Result<String> {
    let s = CStr::from_bytes_until_nul(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "unterminated path"))?;
    match s.to_string_lossy() {
        p if p.is_empty() => Err(io::Error::new(io::ErrorKind::NotFound, "empty path")),
        p => Ok(p.into_owned()),
    }
}

pub(super) fn cwd(pid: u32) -> io::Result<String> {
    let info: libc::proc_vnodepathinfo = pidinfo(pid, libc::PROC_PIDVNODEPATHINFO)?;
    let path = info.pvi_cdir.vip_path.as_flattened();
    // SAFETY: `c_char` and `u8` have the same size and alignment.
    let bytes = unsafe { std::slice::from_raw_parts(path.as_ptr().cast::<u8>(), path.len()) };
    c_string(bytes)
}

pub(super) fn exe(pid: u32) -> io::Result<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the kernel writes at most `buf.len()` bytes.
    let n = unsafe { libc::proc_pidpath(pid as c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n as usize);
    buf.push(0);
    c_string(&buf)
}

/// Pids whose process group is `pgid`; empty when the group has no members.
pub(super) fn group_pids(pgid: u32) -> io::Result<Vec<u32>> {
    list_pids(PROC_PGRP_ONLY, pgid)
}

/// Pids of every process whose uid is `uid`.
pub(super) fn uid_pids(uid: u32) -> io::Result<Vec<u32>> {
    list_pids(PROC_UID_ONLY, uid)
}

fn list_pids(kind: u32, arg: u32) -> io::Result<Vec<u32>> {
    let mut cap = 1024;
    loop {
        let mut buf = vec![0 as c_int; cap];
        let bytes = (cap * size_of::<c_int>()) as c_int;
        // SAFETY: the kernel writes at most `bytes` bytes into `buf`.
        let n = unsafe { libc::proc_listpids(kind, arg, buf.as_mut_ptr().cast(), bytes) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n < bytes {
            buf.truncate(n as usize / size_of::<c_int>());
            return Ok(buf
                .into_iter()
                .filter(|&p| p > 0)
                .map(|p| p as u32)
                .collect());
        }
        cap *= 4; // buffer was full: the list may be truncated
    }
}

/// Replaces `out` with the socket fds of `pid`.
pub(super) fn socket_fds(pid: u32, out: &mut Vec<i32>) -> io::Result<()> {
    let mut buf: Vec<libc::proc_fdinfo> = Vec::new();
    let mut cap = 1024;
    let n = loop {
        buf.resize(
            cap,
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0,
            },
        );
        let bytes = (cap * size_of::<libc::proc_fdinfo>()) as c_int;
        // SAFETY: the kernel writes at most `bytes` bytes into `buf`.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as c_int,
                libc::PROC_PIDLISTFDS,
                0,
                buf.as_mut_ptr().cast(),
                bytes,
            )
        };
        if n <= 0 {
            return Err(io::Error::last_os_error());
        }
        if n < bytes {
            break n as usize / size_of::<libc::proc_fdinfo>();
        }
        cap *= 4; // buffer was full: the list may be truncated
    };
    out.clear();
    out.extend(
        buf[..n]
            .iter()
            .filter(|f| f.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32)
            .map(|f| f.proc_fd),
    );
    Ok(())
}

/// `struct socket_fdinfo` of one socket fd, as raw bytes.
pub(super) fn socket_fdinfo(
    pid: u32,
    fd: i32,
    out: &mut [u8; super::socket::SOCKET_FDINFO_SIZE],
) -> io::Result<()> {
    let size = out.len() as c_int;
    // SAFETY: the kernel writes at most `size` bytes into `out`.
    let n = unsafe {
        libc::proc_pidfdinfo(
            pid as c_int,
            fd,
            PROC_PIDFDSOCKETINFO,
            out.as_mut_ptr().cast(),
            size,
        )
    };
    match n {
        n if n == size => Ok(()),
        n if n <= 0 => Err(io::Error::last_os_error()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short socket_fdinfo",
        )),
    }
}

/// Raw `KERN_PROCARGS2` of `pid`.
pub(super) fn procargs(pid: u32) -> io::Result<Vec<u8>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
    let mut len = 0usize;
    // SAFETY: a null buffer asks the kernel for the size only.
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, null_mut(), &mut len, null_mut(), 0) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; len];
    // SAFETY: the kernel writes at most `len` bytes and updates `len`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(len);
    Ok(buf)
}

/// `mach_timebase_info_data_t`; declared here because libc deprecates its copy.
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> c_int;
}

/// Mach absolute-time ticks (the unit of `proc_taskinfo` CPU times on Apple Silicon) to ns.
pub(super) fn ticks_to_ns(ticks: u64) -> u64 {
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut tb = Timebase { numer: 1, denom: 1 };
        // SAFETY: plain out-parameter.
        let ok = unsafe { mach_timebase_info(&mut tb) } == 0 && tb.denom != 0;
        if ok { (tb.numer, tb.denom) } else { (1, 1) }
    });
    (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64
}

/// argv and initial environment from a `KERN_PROCARGS2` buffer.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Args {
    pub argv: Vec<String>,
    /// `None` when the kernel omitted the environment (restricted binary).
    pub env: Option<BTreeMap<String, String>>,
}

/// Layout: `int argc`, exec path, NUL padding, `argc` strings, env strings, an empty
/// string, then `apple[]` strings. For restricted binaries the buffer ends after argv.
pub(super) fn parse_procargs(buf: &[u8]) -> Option<Args> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?;
    let rest = &buf[4..];
    let mut pos = rest.iter().position(|&b| b == 0)?; // end of exec path
    while rest.get(pos) == Some(&0) {
        pos += 1;
    }
    let mut argv = Vec::with_capacity(argc.min(1024));
    for _ in 0..argc {
        argv.push(String::from_utf8_lossy(take_str(rest, &mut pos)?).into_owned());
    }
    let env = (pos < rest.len()).then(|| {
        let mut env = BTreeMap::new();
        while let Some(s) = take_str(rest, &mut pos).filter(|s| !s.is_empty()) {
            let s = String::from_utf8_lossy(s);
            if let Some((k, v)) = s.split_once('=') {
                env.insert(k.to_owned(), v.to_owned());
            }
        }
        env
    });
    Some(Args { argv, env })
}

/// The NUL-terminated string at `*pos`, advancing past its NUL.
fn take_str<'a>(buf: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let s = buf.get(*pos..)?;
    let end = s.iter().position(|&b| b == 0)?;
    *pos += end + 1;
    Some(&s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(argc: i32, body: &[u8]) -> Vec<u8> {
        let mut b = argc.to_ne_bytes().to_vec();
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn parses_argv_and_env() {
        let b = buf(
            2,
            b"/usr/bin/python3\0\0\0\0python3\0-m\0PORT=5\0A=b=c\0\0ptr_munge=x\0",
        );
        let a = parse_procargs(&b).unwrap();
        assert_eq!(a.argv, ["python3", "-m"]);
        let env = a.env.unwrap();
        assert_eq!(env.len(), 2);
        assert_eq!(env["A"], "b=c");
    }

    #[test]
    fn env_omitted_when_buffer_ends_after_argv() {
        let a = parse_procargs(&buf(1, b"/bin/x\0\0x\0")).unwrap();
        assert_eq!((a.argv.len(), a.env), (1, None));
    }

    #[test]
    fn rejects_truncated_argv() {
        assert_eq!(parse_procargs(&buf(3, b"/bin/x\0x\0")), None);
        assert_eq!(parse_procargs(&[1, 0]), None);
        assert_eq!(parse_procargs(&buf(-1, b"/bin/x\0")), None);
    }
}
