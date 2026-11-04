//! Disk and network byte counters, and turning them into rates.

use serde::Serialize;
use std::collections::HashMap;

/// Cumulative counters of one disk or interface: (read/received, written/sent).
pub type Counters = HashMap<String, (u64, u64)>;

/// `/proc/diskstats`: bytes read and written per device. Sectors are always
/// 512 bytes here, whatever the disk's real sector size.
pub fn parse_diskstats(text: &str) -> Counters {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_ascii_whitespace().collect();
            if f.len() < 10 {
                return None;
            }
            let read: u64 = f[5].parse().ok()?;
            let written: u64 = f[9].parse().ok()?;
            Some((
                f[2].to_string(),
                (read.saturating_mul(512), written.saturating_mul(512)),
            ))
        })
        .collect()
}

/// `/proc/net/dev`: bytes received and sent per interface.
pub fn parse_net_dev(text: &str) -> Counters {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let f: Vec<&str> = rest.split_ascii_whitespace().collect();
            if f.len() < 9 {
                return None;
            }
            Some((
                name.trim().to_string(),
                (f[0].parse().ok()?, f[8].parse().ok()?),
            ))
        })
        .collect()
}

/// Whether a block device counts towards disk I/O: real disks only, so the
/// same I/O is never counted twice through LVM, RAID or partitions.
/// `has_device` is whether `/sys/block/<name>/device` exists.
pub fn is_counted_disk(name: &str, has_device: bool) -> bool {
    const SKIP: &[&str] = &["loop", "ram", "zram", "dm-", "md", "nbd", "sr", "fd"];
    has_device && !SKIP.iter().any(|p| name.starts_with(p))
}

/// What we know about one network interface from `/sys/class/net`.
#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    /// `/sys/class/net/<name>/device` exists: a real network card.
    pub has_device: bool,
}

const VIRTUAL_PREFIXES: &[&str] = &[
    "lo",
    "veth",
    "docker",
    "br-",
    "virbr",
    "vnet",
    "tun",
    "tap",
    "wg",
    "tailscale",
    "zt",
    "flannel",
    "cni",
    "cali",
    "kube",
    "vxlan",
    "genev",
    "dummy",
    "ifb",
    "sit",
    "ip6tnl",
    "ip_vti",
    "gre",
    "erspan",
    "lxcbr",
    "lxdbr",
    "podman",
    "cilium",
    "nebula",
    "ham",
];

/// The interfaces whose counters are added up.
///
/// Real network cards already see everything that goes through bonds,
/// bridges and tunnels, so only they count. Without any (OpenVZ's `venet0`,
/// some Xen guests), everything except loopback and known virtual
/// interfaces counts instead, so the total is never stuck at zero.
pub fn counted_interfaces(all: &[Interface], exclude: &[String]) -> Vec<String> {
    let allowed = |i: &&Interface| {
        !exclude
            .iter()
            .any(|p| super::disks::matches_pattern(p, &i.name))
    };
    let physical: Vec<String> = all
        .iter()
        .filter(allowed)
        .filter(|i| i.has_device)
        .map(|i| i.name.clone())
        .collect();
    if !physical.is_empty() {
        return physical;
    }
    all.iter()
        .filter(allowed)
        .filter(|i| !VIRTUAL_PREFIXES.iter().any(|p| i.name.starts_with(p)))
        .map(|i| i.name.clone())
        .collect()
}

/// Read and write (or receive and send) rates, bytes per second.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct IoRates {
    pub read_bps: u64,
    pub write_bps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NetRates {
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// Bytes moved since the previous sample, summed over `names`.
///
/// Per device: a counter that went down started again from zero (reboot,
/// interface recreated), so its new value is all new traffic. A device seen
/// for the first time adds nothing yet.
pub fn delta(prev: &Counters, now: &Counters, names: &[String]) -> (u64, u64) {
    let mut a = 0u64;
    let mut b = 0u64;
    for name in names {
        let (Some(&(pa, pb)), Some(&(na, nb))) = (prev.get(name), now.get(name)) else {
            continue;
        };
        a = a.saturating_add(if na >= pa { na - pa } else { na });
        b = b.saturating_add(if nb >= pb { nb - pb } else { nb });
    }
    (a, b)
}

pub fn per_second(bytes: u64, secs: f64) -> u64 {
    if secs <= 0.0 {
        0
    } else {
        (bytes as f64 / secs).round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_real_diskstats() {
        let c = parse_diskstats(include_str!("../../fixtures/debian_12/diskstats"));
        assert!(c.contains_key("vda"));
        assert_eq!(c["loop0"], (0, 0));
    }

    #[test]
    fn diskstats_fields_are_sectors() {
        let line = " 253 0 vda 100 5 2048 30 200 7 4096 60 0 90 90\n";
        let c = parse_diskstats(line);
        assert_eq!(c["vda"], (2048 * 512, 4096 * 512));
    }

    #[test]
    fn parses_real_net_dev() {
        let c = parse_net_dev(include_str!("../../fixtures/debian_12/net_dev"));
        assert_eq!(c["eth0"], (110, 42));
        assert_eq!(c["lo"], (0, 0));
    }

    #[test]
    fn net_dev_without_space_after_colon() {
        let c = parse_net_dev("  eth0:123 1 0 0 0 0 0 0 456 2 0 0 0 0 0 0\n");
        assert_eq!(c["eth0"], (123, 456));
    }

    #[test]
    fn only_real_disks_count() {
        assert!(is_counted_disk("vda", true));
        assert!(is_counted_disk("nvme0n1", true));
        assert!(is_counted_disk("sda", true));
        assert!(!is_counted_disk("sda1", false)); // partitions have no device link
        assert!(!is_counted_disk("dm-0", false));
        assert!(!is_counted_disk("md0", false));
        assert!(!is_counted_disk("loop0", false));
        assert!(!is_counted_disk("zram0", true));
        assert!(!is_counted_disk("sr0", true));
    }

    fn iface(name: &str, has_device: bool) -> Interface {
        Interface {
            name: name.into(),
            has_device,
        }
    }

    #[test]
    fn real_cards_only_even_when_bonded_or_bridged() {
        let all = [
            iface("lo", false),
            iface("eth0", true),
            iface("eth1", true),
            iface("bond0", false),
            iface("br0", false),
            iface("docker0", false),
            iface("veth12ab", false),
            iface("wg0", false),
        ];
        assert_eq!(counted_interfaces(&all, &[]), names(&["eth0", "eth1"]));
    }

    #[test]
    fn falls_back_when_no_interface_has_a_device() {
        let all = [
            iface("lo", false),
            iface("venet0", false),
            iface("tun0", false),
        ];
        assert_eq!(counted_interfaces(&all, &[]), names(&["venet0"]));
    }

    #[test]
    fn excluded_interfaces_are_left_out() {
        let all = [iface("eth0", true), iface("eth1", true)];
        assert_eq!(
            counted_interfaces(&all, &names(&["eth1"])),
            names(&["eth0"])
        );
    }

    #[test]
    fn delta_handles_resets_and_new_devices() {
        let prev: Counters = [("a".into(), (100, 100)), ("b".into(), (500, 500))].into();
        let now: Counters = [
            ("a".into(), (150, 130)),
            ("b".into(), (20, 10)),   // reset
            ("c".into(), (999, 999)), // new
        ]
        .into();
        assert_eq!(delta(&prev, &now, &names(&["a", "b", "c"])), (70, 40));
        assert_eq!(delta(&prev, &now, &names(&["a"])), (50, 30));
    }

    #[test]
    fn per_second_rounds() {
        assert_eq!(per_second(1000, 5.0), 200);
        assert_eq!(per_second(1000, 0.0), 0);
    }
}
