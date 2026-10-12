//! Every TCP pcb on the machine from one `net.inet.tcp.pcblist_n` read.
//!
//! The buffer is an `xinpgen` header, then for each pcb a run of records tagged by kind
//! (`xinpcb_n`, `xsocket_n`, two `xsockbuf_n`, `xsockstat_n`, `xtcpcb_n`), then a trailing
//! `xinpgen`. Every record starts with `u32 len, u32 kind` and is padded to 8 bytes, as
//! netstat walks it. Offsets below are from xnu `bsd/netinet/in_pcb.h`,
//! `bsd/sys/socketvar.h` and `bsd/netinet/tcp_var.h`, all under `#pragma pack(4)`.
//!
//! Darwin 27 (macOS 27) filters every `pcblist` sysctl to the calling process's own
//! sockets unless the caller holds the private `com.apple.private.network.statistics`
//! entitlement (as `netstat` does). This read therefore cannot see other processes'
//! listeners there; contract §4.1 needs a decision on the replacement source.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const XSO_SOCKET: u32 = 0x001;
const XSO_INPCB: u32 = 0x010;
const XSO_TCPCB: u32 = 0x020;
const XINPGEN_LEN: usize = 24;

pub(super) const TCPS_LISTEN: i32 = 1;
pub(super) const TCPS_ESTABLISHED: i32 = 4;

// xinpcb_n
const INP_FPORT: usize = 16;
const INP_LPORT: usize = 18;
const INP_VFLAG: usize = 44;
const INP_FADDR: usize = 48;
const INP_LADDR: usize = 64;
const INP_MIN_LEN: usize = 80;
const INP_IPV6: u8 = 0x2;
// xsocket_n
const SO_UID: usize = 64;
const SO_LAST_PID: usize = 68;
const SO_MIN_LEN: usize = 76;
// xtcpcb_n: len, kind, u64 t_segq, int t_dupacks, int t_timer[4], then t_state.
const T_STATE: usize = 36;
const T_MIN_LEN: usize = 40;

/// One TCP pcb: state, endpoints and owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pcb {
    pub state: i32,
    pub local: IpAddr,
    pub lport: u16,
    pub foreign: IpAddr,
    pub fport: u16,
    pub uid: u32,
    /// `so_last_pid`: the last process that did I/O (listen, accept, send) on the socket.
    pub pid: u32,
}

pub(super) fn read() -> io::Result<Vec<Pcb>> {
    parse(&super::sys::sysctl_by_name(c"net.inet.tcp.pcblist_n")?)
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("pcblist_n: {what}"))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn addr(raw: &[u8], v6: bool) -> IpAddr {
    let mut a = [0u8; 16];
    a.copy_from_slice(&raw[..16]);
    if v6 {
        let v6 = Ipv6Addr::from(a);
        v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4)
    } else {
        // in_addr_4in6: three u32 of padding, then the v4 address.
        IpAddr::V4(Ipv4Addr::new(a[12], a[13], a[14], a[15]))
    }
}

#[derive(Default)]
struct Partial {
    inp: Option<(IpAddr, u16, IpAddr, u16)>,
    sock: Option<(u32, u32)>,
    state: Option<i32>,
}

impl Partial {
    fn finish(self) -> Option<Pcb> {
        let (local, lport, foreign, fport) = self.inp?;
        let (uid, pid) = self.sock?;
        Some(Pcb {
            state: self.state?,
            local,
            lport,
            foreign,
            fport,
            uid,
            pid,
        })
    }
}

pub(super) fn parse(buf: &[u8]) -> io::Result<Vec<Pcb>> {
    let header = u32_at(buf, 0).ok_or_else(|| invalid("short header"))? as usize;
    if header < XINPGEN_LEN {
        return Err(invalid("bad header length"));
    }
    let mut pcbs = Vec::new();
    let mut cur = Partial::default();
    let mut off = header.next_multiple_of(8);
    while off < buf.len() {
        let len = u32_at(buf, off).ok_or_else(|| invalid("truncated record"))? as usize;
        if len <= XINPGEN_LEN {
            break; // trailing xinpgen
        }
        let rec = buf
            .get(off..off + len)
            .ok_or_else(|| invalid("record past end"))?;
        let kind = u32_at(rec, 4).ok_or_else(|| invalid("truncated record"))?;
        match kind {
            XSO_INPCB => {
                if len < INP_MIN_LEN {
                    return Err(invalid("short xinpcb_n"));
                }
                pcbs.extend(std::mem::take(&mut cur).finish());
                let v6 = rec[INP_VFLAG] & INP_IPV6 != 0;
                let port = |o: usize| u16::from_be_bytes([rec[o], rec[o + 1]]);
                cur.inp = Some((
                    addr(&rec[INP_LADDR..], v6),
                    port(INP_LPORT),
                    addr(&rec[INP_FADDR..], v6),
                    port(INP_FPORT),
                ));
            }
            XSO_SOCKET => {
                if len < SO_MIN_LEN {
                    return Err(invalid("short xsocket_n"));
                }
                let uid = u32_at(rec, SO_UID).ok_or_else(|| invalid("short xsocket_n"))?;
                let pid = u32_at(rec, SO_LAST_PID).ok_or_else(|| invalid("short xsocket_n"))?;
                cur.sock = Some((uid, pid));
            }
            XSO_TCPCB => {
                let state = u32_at(rec, T_STATE).filter(|_| len >= T_MIN_LEN);
                cur.state = Some(state.ok_or_else(|| invalid("short xtcpcb_n"))? as i32);
            }
            _ => {} // socket buffers and stats
        }
        off += len.next_multiple_of(8);
    }
    pcbs.extend(cur.finish());
    Ok(pcbs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: u32, len: usize, fields: &[(usize, &[u8])]) -> Vec<u8> {
        let mut r = vec![0u8; len.next_multiple_of(8)];
        r[0..4].copy_from_slice(&(len as u32).to_ne_bytes());
        r[4..8].copy_from_slice(&kind.to_ne_bytes());
        for (off, bytes) in fields {
            r[*off..off + bytes.len()].copy_from_slice(bytes);
        }
        r
    }

    fn pcb_records(v6: bool, local: [u8; 16], lport: u16, state: i32, pid: u32) -> Vec<u8> {
        let mut out = record(
            XSO_INPCB,
            104,
            &[
                (INP_LPORT, &lport.to_be_bytes()),
                (INP_FPORT, &0u16.to_be_bytes()),
                (INP_VFLAG, &[if v6 { 2 } else { 1 }]),
                (INP_LADDR, &local),
            ],
        );
        out.extend(record(
            XSO_SOCKET,
            104,
            &[
                (SO_UID, &501u32.to_ne_bytes()),
                (SO_LAST_PID, &pid.to_ne_bytes()),
            ],
        ));
        out.extend(record(0x002, 30, &[])); // rcvbuf, odd length: padded to 8
        out.extend(record(0x004, 30, &[]));
        out.extend(record(0x008, 200, &[]));
        out.extend(record(XSO_TCPCB, 412, &[(T_STATE, &state.to_ne_bytes())]));
        out
    }

    #[test]
    fn parses_v4_and_v6_pcbs() {
        let mut buf = record(0, XINPGEN_LEN, &[]);
        let mut v4 = [0u8; 16];
        v4[12..].copy_from_slice(&[127, 0, 0, 1]);
        buf.extend(pcb_records(false, v4, 8000, TCPS_LISTEN, 42));
        buf.extend(pcb_records(true, [0; 16], 5173, TCPS_ESTABLISHED, 43));
        buf.extend(record(0, XINPGEN_LEN, &[]));

        let pcbs = parse(&buf).unwrap();
        assert_eq!(pcbs.len(), 2);
        assert_eq!(pcbs[0].local, IpAddr::from([127, 0, 0, 1]));
        assert_eq!((pcbs[0].lport, pcbs[0].state, pcbs[0].pid), (8000, 1, 42));
        assert_eq!(pcbs[1].local, IpAddr::V6(Ipv6Addr::UNSPECIFIED));
        assert_eq!((pcbs[1].lport, pcbs[1].uid, pcbs[1].state), (5173, 501, 4));
    }

    #[test]
    fn rejects_truncated_buffers() {
        let mut buf = record(0, XINPGEN_LEN, &[]);
        buf.extend(pcb_records(false, [0; 16], 1, 1, 1));
        buf.truncate(buf.len() - 100);
        assert!(parse(&buf).is_err());
        assert!(parse(&[1, 0]).is_err());
    }
}
