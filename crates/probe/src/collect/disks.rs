//! Disk space: mount points from `/proc/self/mounts`, sizes from `statvfs`.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const MAX_DISKS: usize = 20;
const STATVFS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Disk {
    pub mount: String,
    pub fs: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inodes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inodes_used: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub target: String,
    pub fs: String,
}

/// File systems that do not hold user data.
const PSEUDO_FS: &[&str] = &[
    "autofs",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fusectl",
    "hugetlbfs",
    "iso9660",
    "mqueue",
    "nsfs",
    "overlay",
    "aufs",
    "proc",
    "pstore",
    "ramfs",
    "rootfs",
    "rpc_pipefs",
    "securityfs",
    "selinuxfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
    "nfsd",
];

/// File systems whose `statvfs` can block forever when the server is gone.
const NETWORK_FS: &[&str] = &[
    "9p",
    "afs",
    "ceph",
    "cifs",
    "davfs",
    "fuse",
    "glusterfs",
    "lustre",
    "ncpfs",
    "nfs",
    "nfs4",
    "smb3",
    "smbfs",
    "sshfs",
];

pub fn is_network_fs(fs: &str) -> bool {
    NETWORK_FS.contains(&fs) || (fs.starts_with("fuse.") || fs.starts_with("fuse3."))
}

pub fn is_pseudo_fs(fs: &str) -> bool {
    PSEUDO_FS.contains(&fs)
}

/// Undo the octal escapes the kernel uses for space, tab, newline and `\`.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let v = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(v);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_ascii_whitespace();
            Some(Mount {
                source: unescape(f.next()?),
                target: unescape(f.next()?),
                fs: f.next()?.to_string(),
            })
        })
        .collect()
}

/// `pattern` is an exact path, or a prefix followed by `*`.
pub fn matches_pattern(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

/// The mounts worth calling `statvfs` on, in `/proc/self/mounts` order.
pub fn candidates(mounts: &[Mount], exclude: &[String]) -> Vec<Mount> {
    let mut seen = HashSet::new();
    mounts
        .iter()
        .filter(|m| !is_pseudo_fs(&m.fs) && !is_network_fs(&m.fs))
        .filter(|m| !exclude.iter().any(|p| matches_pattern(p, &m.target)))
        // A later mount on the same path hides the earlier one.
        .rev()
        .filter(|m| seen.insert(m.target.clone()))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .cloned()
        .collect()
}

/// One mount after `statvfs` and `stat`.
#[derive(Debug, Clone)]
pub struct Measured {
    pub mount: Mount,
    pub device: u64,
    pub is_dir: bool,
    pub disk: Disk,
}

/// Drop bind mounts and file mounts, keep at most [`MAX_DISKS`] (block
/// devices first, then the largest) and sort by path.
pub fn select(measured: Vec<Measured>) -> Vec<Disk> {
    let mut seen = HashSet::new();
    let mut kept: Vec<Measured> = measured
        .into_iter()
        .filter(|m| m.is_dir && m.disk.total > 0)
        .filter(|m| seen.insert(m.device))
        .collect();
    kept.sort_by(|a, b| {
        let a_block = a.mount.source.starts_with("/dev/");
        let b_block = b.mount.source.starts_with("/dev/");
        b_block.cmp(&a_block).then(b.disk.total.cmp(&a.disk.total))
    });
    kept.truncate(MAX_DISKS);
    let mut disks: Vec<Disk> = kept.into_iter().map(|m| m.disk).collect();
    disks.sort_by(|a, b| a.mount.cmp(&b.mount));
    disks
}

fn measure_one(mount: &Mount) -> Option<Measured> {
    let st = rustix::fs::stat(mount.target.as_str()).ok()?;
    let vfs = rustix::fs::statvfs(mount.target.as_str()).ok()?;
    let frsize = if vfs.f_frsize > 0 {
        vfs.f_frsize
    } else {
        vfs.f_bsize
    };
    let total = vfs.f_blocks.saturating_mul(frsize);
    let free = vfs.f_bfree.saturating_mul(frsize);
    let available = vfs.f_bavail.saturating_mul(frsize);
    let (inodes_total, inodes_used) = if vfs.f_files > 0 {
        (
            Some(vfs.f_files),
            Some(vfs.f_files.saturating_sub(vfs.f_ffree)),
        )
    } else {
        (None, None)
    };
    let mode = rustix::fs::FileType::from_raw_mode(st.st_mode as _);
    Some(Measured {
        mount: mount.clone(),
        device: st.st_dev as u64,
        is_dir: mode == rustix::fs::FileType::Directory,
        disk: Disk {
            mount: mount.target.clone(),
            fs: mount.fs.clone(),
            total,
            used: total.saturating_sub(free),
            available,
            inodes_total,
            inodes_used,
        },
    })
}

/// Measures disks with a timeout per mount point. A mount whose call has
/// not returned yet is skipped until it does, so a hung mount never piles
/// up threads.
#[derive(Default, Clone)]
pub struct DiskMeter {
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl DiskMeter {
    pub fn measure(&self, mounts_text: &str, exclude: &[String]) -> Vec<Disk> {
        let mut measured = Vec::new();
        for mount in candidates(&parse_mounts(mounts_text), exclude) {
            {
                let mut busy = self.in_flight.lock().unwrap();
                if busy.contains(&mount.target) {
                    continue;
                }
                busy.insert(mount.target.clone());
            }
            let (tx, rx) = mpsc::channel();
            let in_flight = Arc::clone(&self.in_flight);
            let m = mount.clone();
            let spawned = thread::Builder::new()
                .name("statvfs".into())
                .spawn(move || {
                    let result = measure_one(&m);
                    in_flight.lock().unwrap().remove(&m.target);
                    let _ = tx.send(result);
                });
            if spawned.is_err() {
                self.in_flight.lock().unwrap().remove(&mount.target);
                continue;
            }
            match rx.recv_timeout(STATVFS_TIMEOUT) {
                Ok(Some(m)) => measured.push(m),
                Ok(None) => {}
                Err(_) => crate::log::warn(&format!(
                    "statvfs on {} did not return within 2 seconds; skipping it",
                    mount.target
                )),
            }
        }
        select(measured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VPS: &str = "\
/dev/vda1 / ext4 rw,relatime 0 0
proc /proc proc rw 0 0
tmpfs /run tmpfs rw 0 0
/dev/vda15 /boot/efi vfat rw 0 0
/dev/vdb /data xfs rw 0 0
10.0.0.5:/export /mnt/nfs nfs4 rw 0 0
//nas/share /mnt/smb cifs rw 0 0
user@host:/ /mnt/remote fuse.sshfs rw 0 0
/dev/sdc1 /mnt/usb fuseblk rw 0 0
/dev/vdb /srv/bind xfs rw 0 0
/dev/loop3 /snap/core/123 squashfs ro 0 0
/dev/vda1 /mnt/with\\040space ext4 rw 0 0
";

    fn targets(ms: &[Mount]) -> Vec<&str> {
        ms.iter().map(|m| m.target.as_str()).collect()
    }

    #[test]
    fn skips_pseudo_and_network_file_systems() {
        let c = candidates(&parse_mounts(VPS), &[]);
        assert_eq!(
            targets(&c),
            [
                "/",
                "/boot/efi",
                "/data",
                "/mnt/usb",
                "/srv/bind",
                "/mnt/with space"
            ]
        );
    }

    #[test]
    fn honours_excludes() {
        let exclude = vec!["/mnt/*".to_string(), "/boot/efi".to_string()];
        let c = candidates(&parse_mounts(VPS), &exclude);
        assert_eq!(targets(&c), ["/", "/data", "/srv/bind"]);
    }

    #[test]
    fn container_mounts_have_nothing_but_bind_mounted_files() {
        let c = candidates(
            &parse_mounts(include_str!("../../fixtures/debian_12/self_mounts")),
            &[],
        );
        assert!(c.iter().all(|m| m.fs != "overlay" && m.fs != "tmpfs"));
    }

    fn measured(target: &str, source: &str, device: u64, total: u64, is_dir: bool) -> Measured {
        Measured {
            mount: Mount {
                source: source.into(),
                target: target.into(),
                fs: "ext4".into(),
            },
            device,
            is_dir,
            disk: Disk {
                mount: target.into(),
                fs: "ext4".into(),
                total,
                used: 0,
                available: total,
                inodes_total: None,
                inodes_used: None,
            },
        }
    }

    #[test]
    fn select_drops_bind_mounts_and_files() {
        let disks = select(vec![
            measured("/", "/dev/vda1", 1, 100, true),
            measured("/srv/bind", "/dev/vda1", 1, 100, true),
            measured("/etc/hostname", "/dev/vdb1", 2, 50, false),
            measured("/data", "/dev/vdb", 3, 500, true),
        ]);
        let mounts: Vec<_> = disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/data"]);
    }

    #[test]
    fn select_keeps_block_devices_then_largest() {
        let mut all = vec![measured("/zfs", "tank/data", 100, 10_000, true)];
        for i in 0..25 {
            all.push(measured(&format!("/d{i:02}"), "/dev/sdx", i, i + 1, true));
        }
        let disks = select(all);
        assert_eq!(disks.len(), MAX_DISKS);
        assert!(disks.iter().all(|d| d.mount != "/zfs"));
        assert!(disks.iter().all(|d| d.mount.as_str() >= "/d05"));
    }

    #[test]
    fn unescapes_octal() {
        assert_eq!(unescape("/a\\040b\\134c"), "/a b\\c");
        assert_eq!(unescape("/trailing\\04"), "/trailing\\04");
    }

    #[test]
    fn patterns() {
        assert!(matches_pattern("/mnt/*", "/mnt/x"));
        assert!(matches_pattern("/data", "/data"));
        assert!(!matches_pattern("/data", "/data2"));
    }

    #[test]
    fn measures_a_real_directory() {
        let m = measure_one(&Mount {
            source: "/dev/x".into(),
            target: "/".into(),
            fs: "ext4".into(),
        })
        .unwrap();
        assert!(m.is_dir);
        assert!(m.disk.total > 0);
        assert!(m.disk.used + m.disk.available <= m.disk.total + m.disk.total / 10);
    }
}
