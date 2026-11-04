# Metrics

Everything the probe collects, where it comes from and how it is calculated. The probe reads only the files listed here and calls `statvfs` on mount points. It runs no external commands.

Samples are taken every 5 seconds. Each minute's samples are aggregated on the server itself (see [PROTOCOL.md](PROTOCOL.md#minutes)), so the service never stores raw 5-second data.

| Metric | Source | Calculation | Per minute |
|---|---|---|---|
| CPU usage | `/proc/stat`, first line | `1 − (Δidle + Δiowait) / Δtotal`, where total is user + nice + system + idle + iowait + irq + softirq + steal | avg, max |
| CPU iowait | `/proc/stat` | `Δiowait / Δtotal`. Shown separately; not counted as busy | avg, max |
| CPU steal | `/proc/stat` | `Δsteal / Δtotal`. Time the hypervisor gave to other guests | avg, max |
| Load | `/proc/loadavg` | 1, 5 and 15 minute averages as the kernel reports them | avg, max |
| Processes | `/proc` | Number of numeric directories in `/proc`. Only directory names are read, nothing inside them | avg, max |
| Running | `/proc/stat`, `procs_running` | As reported | avg, max |
| Memory total | `/proc/meminfo` | `MemTotal` | last |
| Memory available | `/proc/meminfo` | `MemAvailable`; on kernels older than 3.14, `MemFree + Buffers + Cached` | — |
| Memory used | `/proc/meminfo` | `MemTotal − available`, the same as `free` | avg, max |
| Memory cached | `/proc/meminfo` | `Buffers + Cached + SReclaimable` | avg, max |
| Swap | `/proc/meminfo` | `SwapTotal`, and `SwapTotal − SwapFree` used | last, avg/max |
| Disk space | `/proc/self/mounts`, `statvfs` | total = blocks × fragment size; used = total − free; available = what non-root users can still write. Usage % = used / (used + available), like `df` | last |
| Inodes | `statvfs` | files total and files − free; absent when the total is 0 | last |
| Disk read / write | `/proc/diskstats` | Sum of sectors read / written × 512 over physical disks, per second | avg, max |
| Network receive / send | `/proc/net/dev` | Sum of bytes over physical interfaces, per second; plus bytes per minute | avg, max, total |
| Temperature | `/sys/class/hwmon` | See below | avg, max |
| Uptime, boot time | `/proc/uptime` | Boot time = now − uptime | — |
| System | `/proc/sys/kernel/osrelease`, `/etc/os-release`, `/proc/cpuinfo`, `/proc/stat`, `/sys/firmware/devicetree/base/model` | Kernel, distribution, CPU model and core count | — |
| Virtualization | `/sys/class/dmi/id/sys_vendor`, `product_name`, `/proc/cpuinfo` flags, `/proc/vz`, `/proc/xen`, `/run/systemd/container`, `/.dockerenv`, `/run/.containerenv`, `/proc/1/cgroup` | See below | — |

## Which disks and interfaces count

**Mount points.** Pseudo file systems (`proc`, `sysfs`, `tmpfs`, `overlay`, `squashfs`, `cgroup`, …) are skipped. **Network file systems are skipped** (`nfs`, `cifs`, `smb3`, `sshfs`, `ceph`, `glusterfs`, `9p` and any other `fuse.*` except `fuseblk`): a `statvfs` on a mount whose server is gone can block forever. Every `statvfs` also runs in its own thread with a 2-second timeout; a mount that times out is skipped until the call returns. The same device mounted several times (bind mounts) is counted once. When there are more than 20, block devices come first, then the largest. Mount points can be excluded by name in the config file.

**Disks for read / write.** Only devices in `/sys/block` that have a `device` link, i.e. real disks. Partitions, `dm-*` (LVM, dm-crypt), `md*` (software RAID), `loop`, `ram`, `zram`, `nbd`, `sr` and `fd` are left out, so the same I/O is never counted twice. `/proc/diskstats` always counts in 512-byte sectors, whatever the disk's real sector size.

**Interfaces for network.** Only interfaces in `/sys/class/net` that have a `device` link, i.e. real network cards. Their counters already include traffic that passes through bonds, bridges and VPN tunnels, so virtual interfaces are never added on top. When no interface has a `device` link (OpenVZ's `venet0`, some Xen guests), every interface except `lo` and known virtual ones (`veth*`, `docker*`, `br-*`, `virbr*`, `tun*`, `tap*`, `wg*`, …) is counted instead. Interfaces can be excluded by name in the config file.

**Counters.** Rates are calculated per disk and per interface. A counter that goes down (reboot, interface recreated) is treated as having started again from zero. A disk or interface that appears adds nothing until its second sample.

## Temperature

Sensors are looked up at start and again every hour. The representative value, used for charts and alerts, is the first that exists of:

1. `coretemp` "Package id 0" (Intel)
2. `k10temp` / `zenpower` "Tdie", then "Tctl" (AMD)
3. `cpu_thermal`, `cpu-thermal`, `soc_thermal` (Raspberry Pi and other ARM boards)
4. the highest of all sensors

Readings outside −40 to 150 °C are ignored. Servers without readable sensors (most virtual machines) report no temperature at all.

## Virtualization

Containers are checked first, since a container's DMI data describes its host:

| Sign | Result |
|---|---|
| `/proc/vz` exists and `/proc/bc` does not | container, `openvz` |
| `/run/systemd/container` | container, its content (`lxc`, `docker`, `podman`, `systemd-nspawn`, …) |
| `/.dockerenv` | container, `docker` |
| `/run/.containerenv` | container, `podman` |
| `/proc/1/cgroup` mentions `docker`, `lxc` or `kubepods` | container, that name |
| DMI vendor or product name is a known hypervisor or cloud | vm, its name (`kvm`, `vmware`, `hyperv`, `xen`, `virtualbox`, …) |
| `/proc/xen` exists | vm, `xen` |
| `/proc/cpuinfo` has the `hypervisor` flag | vm, name unknown |
| none of the above | none |

`/sys/class/dmi/id/product_serial` and other root-only DMI files are never read.
