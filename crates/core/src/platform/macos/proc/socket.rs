//! TCP sockets of every same-uid process, from a libproc fd walk: `PROC_PIDLISTFDS`, then
//! `PROC_PIDFDSOCKETINFO` for socket fds only (contract §4.1). The `pcblist_n` sysctl is
//! not usable: since Darwin 27 it only returns the caller's own sockets unless the caller
//! is a platform binary.
//!
//! Offsets into `struct socket_fdinfo` are from the macOS SDK `sys/proc_info.h` (public,
//! stable ABI): `psi` at 24, `soi_kind` at 256, `pri_tcp` at 264.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::sys;

pub(super) const SOCKET_FDINFO_SIZE: usize = 792;
const SOI_KIND: usize = 256;
const SOCKINFO_TCP: u32 = 2;
// tcp_sockinfo.tcpsi_ini (in_sockinfo); ports are ints holding a network-order u16.
const INSI_FPORT: usize = 264;
const INSI_LPORT: usize = 268;
const INSI_VFLAG: usize = 288;
const INSI_FADDR: usize = 296;
const INSI_LADDR: usize = 312;
const TCPSI_STATE: usize = 344;
const INI_IPV6: u8 = 0x2;

pub(super) const TSI_S_LISTEN: i32 = 1;
pub(super) const TSI_S_ESTABLISHED: i32 = 4;

/// One TCP socket held by `pid`. A socket shared by several processes (fork, fd passing)
/// appears once per holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sock {
    pub pid: u32,
    pub state: i32,
    pub local: IpAddr,
    pub lport: u16,
    pub foreign: IpAddr,
    pub fport: u16,
}

/// Every TCP socket of every process of `uid`. Fails only when the process list itself
/// cannot be read.
pub(super) fn tcp_sockets(uid: u32) -> io::Result<Vec<Sock>> {
    let mut socks = Vec::new();
    let mut fds = Vec::new();
    let mut info = [0u8; SOCKET_FDINFO_SIZE];
    for pid in sys::uid_pids(uid)? {
        // ponytail: a same-uid process whose fd list cannot be read (it is exiting) is
        // skipped, so the snapshot has no per-process socket failure (model.rs, §4.8).
        // Upgrade path: report such pids if a real permission failure ever shows up.
        if sys::socket_fds(pid, &mut fds).is_err() {
            continue;
        }
        for &fd in &fds {
            if sys::socket_fdinfo(pid, fd, &mut info).is_ok() {
                socks.extend(parse(pid, &info));
            }
        }
    }
    Ok(socks)
}

fn addr(raw: &[u8], v6: bool) -> IpAddr {
    let mut a = [0u8; 16];
    a.copy_from_slice(&raw[..16]);
    if v6 {
        let v6 = Ipv6Addr::from(a);
        v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4)
    } else {
        // in4in6_addr: three u32 of padding, then the v4 address.
        IpAddr::V4(Ipv4Addr::new(a[12], a[13], a[14], a[15]))
    }
}

/// A TCP socket from a `socket_fdinfo` buffer; `None` for any other socket kind.
fn parse(pid: u32, info: &[u8; SOCKET_FDINFO_SIZE]) -> Option<Sock> {
    let u32_at = |off: usize| u32::from_ne_bytes(info[off..off + 4].try_into().unwrap());
    if u32_at(SOI_KIND) != SOCKINFO_TCP {
        return None;
    }
    let v6 = info[INSI_VFLAG] & INI_IPV6 != 0;
    let port = |off: usize| u16::from_be_bytes([info[off], info[off + 1]]);
    Some(Sock {
        pid,
        state: u32_at(TCPSI_STATE) as i32,
        local: addr(&info[INSI_LADDR..], v6),
        lport: port(INSI_LPORT),
        foreign: addr(&info[INSI_FADDR..], v6),
        fport: port(INSI_FPORT),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(v6: bool, laddr: [u8; 16], lport: u16, state: i32) -> [u8; SOCKET_FDINFO_SIZE] {
        let mut b = [0u8; SOCKET_FDINFO_SIZE];
        b[SOI_KIND..SOI_KIND + 4].copy_from_slice(&SOCKINFO_TCP.to_ne_bytes());
        b[INSI_LPORT..INSI_LPORT + 2].copy_from_slice(&lport.to_be_bytes());
        b[INSI_FPORT..INSI_FPORT + 2].copy_from_slice(&443u16.to_be_bytes());
        b[INSI_VFLAG] = if v6 { 2 } else { 1 };
        b[INSI_LADDR..INSI_LADDR + 16].copy_from_slice(&laddr);
        b[TCPSI_STATE..TCPSI_STATE + 4].copy_from_slice(&state.to_ne_bytes());
        b
    }

    #[test]
    fn parses_v4_v6_and_mapped_addresses() {
        let mut v4 = [0u8; 16];
        v4[12..].copy_from_slice(&[127, 0, 0, 1]);
        let s = parse(7, &info(false, v4, 8000, TSI_S_LISTEN)).unwrap();
        assert_eq!(
            (s.pid, s.local, s.lport, s.fport),
            (7, [127, 0, 0, 1].into(), 8000, 443)
        );
        assert_eq!(s.state, TSI_S_LISTEN);

        let s = parse(7, &info(true, Ipv6Addr::LOCALHOST.octets(), 1, 4)).unwrap();
        assert_eq!(s.local, IpAddr::V6(Ipv6Addr::LOCALHOST));
        let mapped = Ipv4Addr::LOCALHOST.to_ipv6_mapped().octets();
        let s = parse(7, &info(true, mapped, 1, 4)).unwrap();
        assert_eq!(s.local, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn ignores_non_tcp_sockets() {
        let mut b = info(false, [0; 16], 1, 1);
        b[SOI_KIND] = 1; // SOCKINFO_IN (UDP)
        assert_eq!(parse(7, &b), None);
    }
}
