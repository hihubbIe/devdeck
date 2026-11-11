//! Sockets come from `/proc/net/tcp` and `tcp6` (state LISTEN; IPv4-mapped IPv6
//! included; `0.0.0.0` and `127.0.0.1` on the same port conflict). They are matched to
//! processes through `/proc/<pid>/fd` links, for the given pids only (a whole-system
//! scan is thousands of reads). A holder we can't read (another user, docker-proxy as
//! root) is named as such.

use super::{Holder, Listener, Listening};
use crate::{SysError, Support, procs};
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const SUPPORT: Support = Support::Yes;

/// A listening socket from `/proc/net/tcp*`. IPv4-mapped IPv6 addresses are given as
/// IPv4.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Socket {
    addr: IpAddr,
    port: u16,
    inode: u64,
}

/// The LISTEN sockets of a `/proc/net/tcp` or `tcp6` text. Lines are
/// `sl local rem st … uid timeout inode …`; the header and other states are skipped.
fn parse_listen(text: &str) -> Vec<Socket> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Socket> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 10 || fields[3] != "0A" {
        return None;
    }
    let (addr, port) = fields[1].split_once(':')?;
    let inode = fields[9].parse().ok()?;
    Some(Socket { addr: parse_addr(addr)?, port: u16::from_str_radix(port, 16).ok()?, inode })
}

/// The kernel prints an address as 32-bit words in host byte order (`%08X` of each word
/// as stored), so each word's native bytes are the address bytes: `0100007F` is
/// 127.0.0.1 on a little-endian machine.
fn parse_addr(hex: &str) -> Option<IpAddr> {
    let word = |i: usize| hex.get(i * 8..i * 8 + 8).and_then(|w| u32::from_str_radix(w, 16).ok()).map(u32::to_ne_bytes);
    match hex.len() {
        8 => Some(IpAddr::V4(Ipv4Addr::from(word(0)?))),
        32 => {
            let mut bytes = [0u8; 16];
            for i in 0..4 {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&word(i)?);
            }
            let v6 = Ipv6Addr::from(bytes);
            Some(v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4))
        }
        _ => None,
    }
}

/// Every LISTEN socket on this machine (in our network namespace), IPv4 and IPv6. Only
/// an unreadable `/proc/net/tcp` is an error: `tcp6` is missing where IPv6 is off.
fn listen_sockets() -> Result<Vec<Socket>, SysError> {
    let mut out = parse_listen(&std::fs::read_to_string("/proc/net/tcp")?);
    if let Ok(t) = std::fs::read_to_string("/proc/net/tcp6") {
        out.extend(parse_listen(&t));
    }
    Ok(out)
}

/// The socket inodes `pid` holds open (`socket:[N]` fd links). None when its fds can't
/// be read (another user's process, or gone).
fn socket_inodes(pid: u32) -> Option<Vec<u64>> {
    let dir = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    Some(dir.filter_map(|e| parse_socket_link(std::fs::read_link(e.ok()?.path()).ok()?.to_str()?)).collect())
}

fn parse_socket_link(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

pub fn listening(pids: &[u32]) -> Result<Vec<Listening>, SysError> {
    let ports: HashMap<u64, (u16, IpAddr)> = listen_sockets()?.into_iter().map(|s| (s.inode, (s.port, s.addr))).collect();
    if ports.is_empty() {
        return Ok(Vec::new());
    }
    let ports = &ports;
    let found: BTreeSet<Listening> = pids
        .iter()
        .flat_map(|&pid| {
            socket_inodes(pid).unwrap_or_default().into_iter().filter_map(move |i| {
                let (port, addr) = *ports.get(&i)?;
                Some(Listening { port, pid, addr })
            })
        })
        .collect();
    Ok(found.into_iter().collect())
}

/// The topmost of `pids` (none of whose parents is among them): the one to stop.
fn topmost(pids: &BTreeSet<u32>) -> Option<u32> {
    pids.iter().find(|&&pid| procs::parent_of(pid).is_none_or(|p| !pids.contains(&p))).or(pids.first()).copied()
}

fn process(pid: u32) -> Holder {
    Holder::Process { pid, command: procs::command(pid).or_else(|| procs::name(pid)).unwrap_or_default() }
}

pub fn all() -> Result<Vec<Listener>, SysError> {
    let sockets = listen_sockets()?;
    let wanted: BTreeSet<u64> = sockets.iter().map(|s| s.inode).collect();
    // Who holds each socket: every readable process's descriptors, once.
    let mut holders: HashMap<u64, BTreeSet<u32>> = HashMap::new();
    if !wanted.is_empty()
        && let Ok(dir) = std::fs::read_dir("/proc")
    {
        for pid in dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok()) {
            for inode in socket_inodes(pid).unwrap_or_default().into_iter().filter(|i| wanted.contains(i)) {
                holders.entry(inode).or_default().insert(pid);
            }
        }
    }
    let mut seen = BTreeSet::new();
    Ok(sockets
        .into_iter()
        .filter(|s| seen.insert((s.port, s.addr)))
        .map(|s| Listener { port: s.port, addr: s.addr, holder: holders.get(&s.inode).and_then(topmost).map_or(Holder::Unknown, process) })
        .collect())
}

pub fn holder(port: u16) -> Result<Option<Holder>, SysError> {
    let inodes: BTreeSet<u64> = listen_sockets()?.into_iter().filter(|s| s.port == port).map(|s| s.inode).collect();
    if inodes.is_empty() {
        return Ok(None);
    }
    let Ok(dir) = std::fs::read_dir("/proc") else { return Ok(Some(Holder::Unknown)) };
    let holders: BTreeSet<u32> = dir
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| socket_inodes(pid).is_some_and(|own| own.iter().any(|i| inodes.contains(i))))
        .collect();
    Ok(Some(topmost(&holders).map_or(Holder::Unknown, process)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0000000000000000 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 17 1 0000000000000000 100 0 0 10 0
   2: 0100007F:1F90 0100007F:D431 01 00000000:00000000 00:00000000 00000000  1000        0 999 1 0000000000000000 20 4 30 10 -1
";

    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:0BB8 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 51 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:0050 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 52 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF00000100007F:1389 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 53 1 0000000000000000 100 0 0 10 0
   3: 00000000000000000000000001000000:0BB8 00000000000000000000000001000000:9C40 06 00000000:00000000 03:00000000 00000000     0        0 0 3 0000000000000000
";

    #[test]
    fn ipv4_listen_lines_are_parsed_and_other_states_skipped() {
        assert_eq!(
            parse_listen(TCP),
            [
                Socket { addr: IpAddr::V4(Ipv4Addr::LOCALHOST), port: 8080, inode: 4242 },
                Socket { addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED), port: 22, inode: 17 },
            ]
        );
    }

    #[test]
    fn ipv6_listen_lines_are_parsed_and_mapped_addresses_are_ipv4() {
        assert_eq!(
            parse_listen(TCP6),
            [
                Socket { addr: IpAddr::V6(Ipv6Addr::LOCALHOST), port: 3000, inode: 51 },
                Socket { addr: IpAddr::V6(Ipv6Addr::UNSPECIFIED), port: 80, inode: 52 },
                Socket { addr: IpAddr::V4(Ipv4Addr::LOCALHOST), port: 5001, inode: 53 },
            ]
        );
    }

    #[test]
    fn garbage_and_socket_links() {
        assert_eq!(parse_listen("nonsense\n0: zz:1 0:0 0A a b c d e f\n"), []);
        assert_eq!(parse_socket_link("socket:[12345]"), Some(12345));
        assert_eq!(parse_socket_link("pipe:[12345]"), None);
        assert_eq!(parse_socket_link("/dev/null"), None);
    }
}
