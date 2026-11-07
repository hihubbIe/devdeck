//! Listening ports (R6.9): which ports a run's processes listen on, who holds a port
//! before a start, and the ports each command used last time.
//!
//! Sockets come from `/proc/net/tcp` and `tcp6` (state LISTEN; IPv4-mapped IPv6
//! included; `0.0.0.0` and `127.0.0.1` on the same port conflict). They are matched to
//! processes through `/proc/<pid>/fd` links, for the given pids only (a whole-system
//! scan is thousands of reads). A holder we can't read (another user, docker-proxy as
//! root) is named as such.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Listening {
    pub port: u16,
    pub pid: u32,
}

/// Who holds a port.
#[derive(Clone, Debug, PartialEq)]
pub enum Holder {
    /// A process we can see: its pid and command line.
    Process { pid: u32, command: String },
    /// Listening, but the process isn't readable (another user).
    Unknown,
}

/// A listening socket from `/proc/net/tcp*`. IPv4-mapped IPv6 addresses are given as
/// IPv4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Socket {
    pub addr: IpAddr,
    pub port: u16,
    pub inode: u64,
}

/// The LISTEN sockets of a `/proc/net/tcp` or `tcp6` text. Lines are
/// `sl local rem st … uid timeout inode …`; the header and other states are skipped.
pub fn parse_listen(text: &str) -> Vec<Socket> {
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

/// Every LISTEN socket on this machine (in our network namespace), IPv4 and IPv6.
fn listen_sockets() -> Vec<Socket> {
    ["/proc/net/tcp", "/proc/net/tcp6"].iter().filter_map(|p| std::fs::read_to_string(p).ok()).flat_map(|t| parse_listen(&t)).collect()
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

/// The ports `pids` listen on.
pub fn listening(pids: &[u32]) -> Vec<Listening> {
    let ports: HashMap<u64, u16> = listen_sockets().into_iter().map(|s| (s.inode, s.port)).collect();
    if ports.is_empty() {
        return Vec::new();
    }
    let ports = &ports;
    let found: BTreeSet<Listening> = pids
        .iter()
        .flat_map(|&pid| socket_inodes(pid).unwrap_or_default().into_iter().filter_map(move |i| Some(Listening { port: *ports.get(&i)?, pid })))
        .collect();
    found.into_iter().collect()
}

/// Who listens on `port` now, if anyone.
///
/// Scans every readable process (called on demand before a start, not per frame). When
/// several share the socket (a server's workers inherit it), the topmost one is named:
/// the one to stop.
pub fn holder(port: u16) -> Option<Holder> {
    let inodes: BTreeSet<u64> = listen_sockets().into_iter().filter(|s| s.port == port).map(|s| s.inode).collect();
    if inodes.is_empty() {
        return None;
    }
    let Ok(dir) = std::fs::read_dir("/proc") else { return Some(Holder::Unknown) };
    let holders: BTreeSet<u32> = dir
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| socket_inodes(pid).is_some_and(|own| own.iter().any(|i| inodes.contains(i))))
        .collect();
    let top = holders.iter().find(|&&pid| crate::procfs::parent_of(pid).is_none_or(|p| !holders.contains(&p))).or(holders.first());
    Some(top.map_or(Holder::Unknown, |&pid| Holder::Process { pid, command: command_of(pid) }))
}

/// A process's command line (NUL-separated arguments joined by spaces), else its name.
fn command_of(pid: u32) -> String {
    let args = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let line = args.split(|&b| b == 0).filter(|a| !a.is_empty()).map(String::from_utf8_lossy).collect::<Vec<_>>().join(" ");
    if !line.is_empty() {
        return line;
    }
    std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|c| c.trim_end().to_string()).unwrap_or_default()
}

/// Ports each command listened on last time, in the state folder: a conflict can be
/// reported before a start even when no config names the port. Nested by workspace id,
/// app id, command id (app ids hold `/`, so no joined key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortMemory {
    pub path: PathBuf,
    pub ports: BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<u16>>>>,
}

impl PortMemory {
    /// Reads `ports.json` in `state_dir`. Missing or unreadable means nothing
    /// remembered; a broken file is moved aside (`ports.json.bad`) so the next save
    /// doesn't destroy it.
    pub fn load(state_dir: &Path) -> PortMemory {
        let path = state_dir.join("ports.json");
        let ports = match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| {
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                BTreeMap::new()
            }),
            Err(_) => BTreeMap::new(),
        };
        PortMemory { path, ports }
    }

    pub fn get(&self, workspace: &str, app: &str, cmd: &str) -> &[u16] {
        self.ports.get(workspace).and_then(|a| a.get(app)).and_then(|c| c.get(cmd)).map_or(&[], Vec::as_slice)
    }

    /// Saves at once when they changed. Ports are kept sorted without duplicates; no
    /// ports forgets the command.
    pub fn remember(&mut self, workspace: &str, app: &str, cmd: &str, ports: &[u16]) -> Result<(), String> {
        let ports: Vec<u16> = ports.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
        if self.get(workspace, app, cmd) == ports.as_slice() {
            return Ok(());
        }
        if ports.is_empty() {
            let apps = self.ports.get_mut(workspace).expect("known, since it changed");
            let cmds = apps.get_mut(app).expect("known, since it changed");
            cmds.remove(cmd);
            if cmds.is_empty() {
                apps.remove(app);
            }
            if apps.is_empty() {
                self.ports.remove(workspace);
            }
        } else {
            self.ports.entry(workspace.into()).or_default().entry(app.into()).or_default().insert(cmd.into(), ports);
        }
        let text = serde_json::to_string_pretty(&self.ports).map_err(|e| e.to_string())?;
        crate::state::write_atomic(&self.path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

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

    fn found_as_this_process(l: &TcpListener) {
        let port = l.local_addr().unwrap().port();
        let me = std::process::id();
        assert!(listening(&[me]).contains(&Listening { port, pid: me }), "port {port} found for this process");
        match holder(port) {
            Some(Holder::Process { pid, command }) => {
                assert_eq!(pid, me);
                assert!(command.contains("devdeck_core"), "command: {command}");
            }
            other => panic!("holder of {port}: {other:?}"),
        }
    }

    #[test]
    fn a_real_ipv4_listener_is_found() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        found_as_this_process(&l);
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert_eq!(holder(port), None, "nobody listens once closed");
    }

    #[test]
    fn a_real_ipv6_listener_is_found_when_ipv6_is_available() {
        let Ok(l) = TcpListener::bind("[::1]:0") else { return };
        found_as_this_process(&l);
    }

    #[test]
    fn other_pids_are_not_scanned_for_our_ports() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let parent = crate::procfs::parent_of(std::process::id()).unwrap();
        assert!(!listening(&[parent]).iter().any(|x| x.port == port));
        assert_eq!(listening(&[]), []);
    }

    fn temp_dir(tag: &str) -> crate::testing::TempDir {
        crate::testing::TempDir::new(tag)
    }

    #[test]
    fn port_memory_round_trip() {
        let dir = temp_dir("memory");
        let mut m = PortMemory::load(&dir);
        assert_eq!(m.get("work", "apps/web", "dev"), [] as [u16; 0]);
        m.remember("work", "apps/web", "dev", &[5173, 3000, 5173]).unwrap();
        m.remember("work", "api", "start", &[8080]).unwrap();
        assert_eq!(m.get("work", "apps/web", "dev"), [3000, 5173], "sorted, no duplicates");
        let back = PortMemory::load(&dir);
        assert_eq!(back, m);

        std::fs::remove_file(&m.path).unwrap();
        m.remember("work", "apps/web", "dev", &[3000, 5173]).unwrap();
        assert!(!m.path.exists(), "unchanged ports are not saved again");

        m.remember("work", "apps/web", "dev", &[]).unwrap();
        assert_eq!(PortMemory::load(&dir).ports, BTreeMap::from([("work".into(), BTreeMap::from([("api".into(), BTreeMap::from([("start".into(), vec![8080])]))]))]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_port_memory_is_empty_and_kept_aside() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ports.json"), "{oops").unwrap();
        let m = PortMemory::load(&dir);
        assert!(m.ports.is_empty());
        assert!(dir.join("ports.json.bad").exists(), "broken file kept aside");
        assert!(!dir.join("ports.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
