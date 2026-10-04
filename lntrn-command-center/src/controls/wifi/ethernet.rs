//! Wired (Ethernet) port state for the network panel.
//!
//! Nothing here talks to a daemon. The kernel already publishes all of
//! it, readable without privileges:
//! - `/sys/class/net/<if>/…`: which interfaces are physical wired ports,
//!   whether they're switched on, carrier, link speed.
//! - `getifaddrs(3)`: the addresses on them.
//! - `/proc/net/route` + `/proc/net/ipv6_route`: which interface owns
//!   the default route, i.e. which connection traffic actually takes.
//!
//! There is deliberately no "connect" action. The system's network
//! daemon (dhcpcd on the Gentoo desktop) configures a wired port the
//! moment it links and ranks it above wireless, so the panel's job is
//! to show what the port is doing, not to drive it.

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use super::Wifi;

const SYS_NET: &str = "/sys/class/net";
/// `ARPHRD_ETHER`, the `type` every Ethernet-framed interface reports.
const ARPHRD_ETHER: &str = "1";
const IFF_UP: u32 = 0x1;
const RTF_UP: u32 = 0x0001;
const RTF_REJECT: u32 = 0x0200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EthLink {
    /// The interface is administratively down: the port is switched off,
    /// so a plugged-in cable can't show up as a link.
    Disabled,
    /// The port is on but nothing answers on the cable.
    NoLink,
    /// Link established with whatever is on the far end.
    Up,
}

/// One physical wired port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EthPort {
    /// Kernel interface name, e.g. "eno1".
    pub name: String,
    pub link: EthLink,
    /// Negotiated link speed in Mbit/s. `None` without a link, and on
    /// drivers that don't report one.
    pub speed_mbps: Option<u32>,
    /// IPv4 address if the port has one, else a routable IPv6 one.
    pub address: Option<String>,
    /// This port owns the default route.
    pub default_route: bool,
}

impl EthPort {
    /// Traffic is going over this port right now.
    pub fn in_use(&self) -> bool {
        self.link == EthLink::Up && self.default_route
    }
}

impl Wifi {
    /// Wired ports on this machine, sorted by name. Empty when it has none.
    pub fn ethernet(&self) -> &[EthPort] {
        &self.ethernet
    }

    /// True when a wired port, not WiFi, is carrying the traffic.
    pub fn wired_active(&self) -> bool {
        self.ethernet.iter().any(EthPort::in_use)
    }
}

/// Snapshot every wired port. A handful of small procfs / sysfs reads,
/// but still file I/O: call it from the worker thread, not per frame.
pub(crate) fn poll() -> Vec<EthPort> {
    let mut names = wired_interfaces();
    if names.is_empty() {
        return Vec::new();
    }
    names.sort();

    let route_iface = default_route_iface();
    let addresses = addresses();
    names
        .into_iter()
        .map(|name| {
            let dir = Path::new(SYS_NET).join(&name);
            let link = read_link(&dir);
            let speed_mbps = if link == EthLink::Up {
                // "-1" when the driver has nothing to report.
                read_trimmed(&dir.join("speed"))
                    .and_then(|s| s.parse::<i64>().ok())
                    .and_then(|s| u32::try_from(s).ok())
                    .filter(|s| *s > 0)
            } else {
                None
            };
            EthPort {
                address: addresses.get(&name).map(|a| a.to_string()),
                default_route: route_iface.as_deref() == Some(name.as_str()),
                name,
                link,
                speed_mbps,
            }
        })
        .collect()
}

/// Names of the physical wired ports. Ethernet-framed, backed by real
/// hardware (`device` is missing on bridges, veth pairs, tunnels…), and
/// not a typed subclass: WiFi, WWAN, bonds and friends all set a
/// `DEVTYPE` in their uevent, plain Ethernet sets none.
fn wired_interfaces() -> Vec<String> {
    let Ok(entries) = fs::read_dir(SYS_NET) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            let dir = entry.path();
            read_trimmed(&dir.join("type")).as_deref() == Some(ARPHRD_ETHER)
                && dir.join("device").exists()
                && fs::read_to_string(dir.join("uevent"))
                    .is_ok_and(|uevent| !uevent.lines().any(|l| l.starts_with("DEVTYPE=")))
        })
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

fn read_link(dir: &Path) -> EthLink {
    let up = read_trimmed(&dir.join("flags"))
        .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .is_some_and(|flags| flags & IFF_UP != 0);
    if !up {
        // `carrier` can't be read at all while the interface is down.
        return EthLink::Disabled;
    }
    match read_trimmed(&dir.join("carrier")).as_deref() {
        Some("1") => EthLink::Up,
        _ => EthLink::NoLink,
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// One address per interface: IPv4 when there is one, otherwise the
/// first IPv6 address that isn't link-local (those appear the moment a
/// link comes up and say nothing about the port being configured).
fn addresses() -> HashMap<String, IpAddr> {
    let mut out: HashMap<String, IpAddr> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: on success getifaddrs points `head` at a list that stays
    // valid until the freeifaddrs below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the live list.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: `ifa_addr` is non-null and at least a `sockaddr`; its
        // family says which concrete struct it really is.
        let addr = match unsafe { (*ifa.ifa_addr).sa_family } as i32 {
            libc::AF_INET => {
                let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
            }
            libc::AF_INET6 => {
                let sin6 = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                let v6 = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                if v6.segments()[0] & 0xffc0 == 0xfe80 {
                    continue;
                }
                IpAddr::V6(v6)
            }
            _ => continue,
        };
        // SAFETY: `ifa_name` is a non-null NUL-terminated string.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let keep_existing = match out.get(&name) {
            Some(IpAddr::V4(_)) => true,
            Some(IpAddr::V6(_)) => addr.is_ipv6(),
            None => false,
        };
        if !keep_existing {
            out.insert(name, addr);
        }
    }
    // SAFETY: `head` came from a successful getifaddrs and nothing
    // borrowed from the list outlives this point.
    unsafe { libc::freeifaddrs(head) };
    out
}

/// Interface carrying the default route: the lowest-metric IPv4 default,
/// or the IPv6 one on a network with no IPv4 at all.
fn default_route_iface() -> Option<String> {
    fs::read_to_string("/proc/net/route")
        .ok()
        .and_then(|table| default_route_v4(&table))
        .or_else(|| {
            fs::read_to_string("/proc/net/ipv6_route")
                .ok()
                .and_then(|table| default_route_v6(&table))
        })
}

/// `/proc/net/route` columns: Iface Destination Gateway Flags RefCnt Use
/// Metric Mask … (addresses and flags in hex, metric in decimal).
fn default_route_v4(table: &str) -> Option<String> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let iface = cols.next()?;
            let dest = cols.next()?;
            let flags = u32::from_str_radix(cols.nth(1)?, 16).ok()?;
            let metric: u32 = cols.nth(2)?.parse().ok()?;
            let mask = cols.next()?;
            (dest == "00000000" && mask == "00000000" && flags & RTF_UP != 0)
                .then(|| (metric, iface.to_string()))
        })
        .min()
        .map(|(_, iface)| iface)
}

/// `/proc/net/ipv6_route` columns: dest dest-prefix-len src src-prefix-len
/// next-hop metric refcnt use flags iface (all hex). The kernel also
/// lists an unreachable catch-all default on `lo`; RTF_REJECT drops it.
fn default_route_v6(table: &str) -> Option<String> {
    table
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let dest = cols.next()?;
            let prefix_len = cols.next()?;
            let metric = u32::from_str_radix(cols.nth(3)?, 16).ok()?;
            let flags = u32::from_str_radix(cols.nth(2)?, 16).ok()?;
            let iface = cols.next()?;
            (prefix_len == "00"
                && dest.bytes().all(|b| b == b'0')
                && flags & RTF_UP != 0
                && flags & RTF_REJECT == 0)
                .then(|| (metric, iface.to_string()))
        })
        .min()
        .map(|(_, iface)| iface)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE_HEADER: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";

    #[test]
    fn v4_default_goes_to_the_lowest_metric() {
        let table = format!(
            "{ROUTE_HEADER}\
             wlan0\t00000000\t0100000A\t0003\t0\t0\t3005\t00000000\t0\t0\t0\n\
             eno1\t00000000\t0100000A\t0003\t0\t0\t1002\t00000000\t0\t0\t0\n\
             eno1\t0000000A\t00000000\t0001\t0\t0\t1002\t00FFFFFF\t0\t0\t0\n"
        );
        assert_eq!(default_route_v4(&table).as_deref(), Some("eno1"));
    }

    #[test]
    fn v4_subnet_routes_are_not_defaults() {
        let table = format!(
            "{ROUTE_HEADER}\
             eno1\t0000000A\t00000000\t0001\t0\t0\t1002\t00FFFFFF\t0\t0\t0\n"
        );
        assert_eq!(default_route_v4(&table), None);
    }

    #[test]
    fn v6_default_skips_the_unreachable_catch_all() {
        let zero = "0".repeat(32);
        let gateway = "fe800000000000000aa7c0fffe2ba5dd";
        let table = format!(
            "{zero} 00 {zero} 00 {zero} ffffffff 00000001 00000000 00200200       lo\n\
             26010000000000000000000000000000 40 {zero} 00 {zero} 00000bbd 00000001 00000000 00400001    wlan0\n\
             {zero} 00 {zero} 00 {gateway} 00000bbd 00000007 00000000 00400003    wlan0\n"
        );
        assert_eq!(default_route_v6(&table).as_deref(), Some("wlan0"));
        let only_reject =
            format!("{zero} 00 {zero} 00 {zero} ffffffff 00000001 00000000 00200200       lo\n");
        assert_eq!(default_route_v6(&only_reject), None);
    }
}
