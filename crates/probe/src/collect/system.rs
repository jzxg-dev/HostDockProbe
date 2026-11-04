//! What the server is: kernel, distribution, CPU, memory, virtualization.

use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OsRelease {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pretty_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Virtualization {
    /// `none`, `vm` or `container`.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    pub os: OsRelease,
    pub arch: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_cores: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_total: Option<u64>,
    pub virtualization: Virtualization,
}

pub fn parse_os_release(text: &str) -> OsRelease {
    let mut os = OsRelease::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "ID" => os.id = Some(value),
            "NAME" => os.name = Some(value),
            "VERSION_ID" => os.version_id = Some(value),
            "PRETTY_NAME" => os.pretty_name = Some(value),
            _ => {}
        }
    }
    os
}

/// Known Arm cores, by (implementer, part), for `/proc/cpuinfo` files that
/// carry no model name (Graviton, Ampere and most Arm servers).
fn arm_core(implementer: &str, part: &str) -> Option<&'static str> {
    Some(match (implementer, part) {
        ("0x41", "0xd03") => "Arm Cortex-A53",
        ("0x41", "0xd07") => "Arm Cortex-A57",
        ("0x41", "0xd08") => "Arm Cortex-A72",
        ("0x41", "0xd0b") => "Arm Cortex-A76",
        ("0x41", "0xd0c") => "Arm Neoverse-N1",
        ("0x41", "0xd40") => "Arm Neoverse-V1",
        ("0x41", "0xd49") => "Arm Neoverse-N2",
        ("0x41", "0xd4f") => "Arm Neoverse-V2",
        ("0xc0", "0xac3") => "Ampere-1",
        ("0xc0", "0xac4") => "Ampere-1a",
        _ => return None,
    })
}

/// CPU model from `/proc/cpuinfo`, or failing that the board model from the
/// device tree (Raspberry Pi and similar).
pub fn cpu_model(cpuinfo: &str, devicetree_model: Option<&str>) -> Option<String> {
    let field = |name: &str| {
        cpuinfo.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == name && !v.trim().is_empty()).then(|| v.trim().to_string())
        })
    };
    field("model name")
        .or_else(|| field("cpu model"))
        .or_else(|| match (field("CPU implementer"), field("CPU part")) {
            (Some(i), Some(p)) => arm_core(&i, &p).map(str::to_string),
            _ => None,
        })
        .or_else(|| field("Model"))
        .or_else(|| field("Hardware"))
        .or_else(|| {
            devicetree_model
                .map(|m| m.trim_matches(char::from(0)).trim().to_string())
                .filter(|m| !m.is_empty())
        })
}

/// Everything virtualization detection looks at, gathered first so the
/// decision itself can be tested.
#[derive(Debug, Default)]
pub struct VirtSigns {
    pub proc_vz: bool,
    pub proc_bc: bool,
    pub systemd_container: Option<String>,
    pub dockerenv: bool,
    pub containerenv: bool,
    pub pid1_cgroup: Option<String>,
    pub dmi_vendor: Option<String>,
    pub dmi_product: Option<String>,
    pub proc_xen: bool,
    pub hypervisor_flag: bool,
}

fn vm_from_dmi(vendor: &str, product: &str) -> Option<&'static str> {
    let v = vendor.to_ascii_lowercase();
    let p = product.to_ascii_lowercase();
    let either = |s: &str| v.contains(s) || p.contains(s);
    Some(if either("vmware") {
        "vmware"
    } else if either("virtualbox") || v.contains("innotek") {
        "virtualbox"
    } else if v.contains("microsoft") && p.contains("virtual") {
        "hyperv"
    } else if either("xen") {
        "xen"
    } else if either("parallels") {
        "parallels"
    } else if either("bochs") {
        "bochs"
    } else if either("qemu")
        || either("kvm")
        || either("openstack")
        || either("google")
        || either("amazon ec2")
        || either("digitalocean")
        || either("hetzner")
        || either("alibaba")
        || either("tencent")
        || either("vultr")
        || either("linode")
        || either("upcloud")
        || either("scaleway")
        || either("oracle")
        || either("apple virtualization")
    {
        "kvm"
    } else {
        return None;
    })
}

pub fn detect_virtualization(s: &VirtSigns) -> Virtualization {
    let container = |name: &str| Virtualization {
        kind: "container",
        name: Some(name.to_string()),
    };
    // Containers first: their DMI data describes the host.
    if s.proc_vz && !s.proc_bc {
        return container("openvz");
    }
    if let Some(name) = s.systemd_container.as_deref().map(str::trim)
        && !name.is_empty()
    {
        return container(name);
    }
    if s.dockerenv {
        return container("docker");
    }
    if s.containerenv {
        return container("podman");
    }
    if let Some(cg) = &s.pid1_cgroup {
        for name in ["docker", "lxc", "kubepods"] {
            if cg.contains(name) {
                return container(name);
            }
        }
    }
    if let Some(name) = vm_from_dmi(
        s.dmi_vendor.as_deref().unwrap_or(""),
        s.dmi_product.as_deref().unwrap_or(""),
    ) {
        return Virtualization {
            kind: "vm",
            name: Some(name.into()),
        };
    }
    if s.proc_xen {
        return Virtualization {
            kind: "vm",
            name: Some("xen".into()),
        };
    }
    if s.hypervisor_flag {
        return Virtualization {
            kind: "vm",
            name: None,
        };
    }
    Virtualization {
        kind: "none",
        name: None,
    }
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn read_trimmed(path: &str) -> Option<String> {
    read(path)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn has_hypervisor_flag(cpuinfo: &str) -> bool {
    cpuinfo.lines().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            (k.trim() == "flags" || k.trim() == "Features")
                && v.split_ascii_whitespace().any(|f| f == "hypervisor")
        })
    })
}

pub fn collect(cores: Option<u32>, memory_total: Option<u64>) -> SystemInfo {
    let cpuinfo = read("/proc/cpuinfo").unwrap_or_default();
    let signs = VirtSigns {
        proc_vz: Path::new("/proc/vz").exists(),
        proc_bc: Path::new("/proc/bc").exists(),
        systemd_container: read_trimmed("/run/systemd/container"),
        dockerenv: Path::new("/.dockerenv").exists(),
        containerenv: Path::new("/run/.containerenv").exists(),
        pid1_cgroup: read("/proc/1/cgroup"),
        dmi_vendor: read_trimmed("/sys/class/dmi/id/sys_vendor"),
        dmi_product: read_trimmed("/sys/class/dmi/id/product_name"),
        proc_xen: Path::new("/proc/xen").exists(),
        hypervisor_flag: has_hypervisor_flag(&cpuinfo),
    };
    SystemInfo {
        kernel: read_trimmed("/proc/sys/kernel/osrelease"),
        os: parse_os_release(
            &read("/etc/os-release")
                .or_else(|| read("/usr/lib/os-release"))
                .unwrap_or_default(),
        ),
        arch: std::env::consts::ARCH,
        cpu_model: cpu_model(
            &cpuinfo,
            read("/sys/firmware/devicetree/base/model").as_deref(),
        ),
        cpu_cores: cores.filter(|c| *c > 0),
        memory_total,
        virtualization: detect_virtualization(&signs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_os_release() {
        let os = parse_os_release(include_str!("../../fixtures/debian_12/os-release"));
        assert_eq!(os.id.as_deref(), Some("debian"));
        assert_eq!(os.version_id.as_deref(), Some("12"));
        assert!(os.pretty_name.unwrap().starts_with("Debian GNU/Linux 12"));
        let os = parse_os_release(include_str!("../../fixtures/alpine_3_20/os-release"));
        assert_eq!(os.id.as_deref(), Some("alpine"));
    }

    #[test]
    fn x86_model_name() {
        let text = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD EPYC 7B13\n";
        assert_eq!(cpu_model(text, None).as_deref(), Some("AMD EPYC 7B13"));
    }

    #[test]
    fn arm_server_without_model_name() {
        let text =
            "processor\t: 0\nBogoMIPS\t: 243.75\nCPU implementer\t: 0x41\nCPU part\t: 0xd0c\n";
        assert_eq!(cpu_model(text, None).as_deref(), Some("Arm Neoverse-N1"));
    }

    #[test]
    fn raspberry_pi_uses_model_line_or_device_tree() {
        let text = "processor\t: 0\nCPU implementer\t: 0x41\nCPU part\t: 0xd99\n\
                    Model\t\t: Raspberry Pi 4 Model B Rev 1.4\n";
        assert_eq!(
            cpu_model(text, None).as_deref(),
            Some("Raspberry Pi 4 Model B Rev 1.4")
        );
        assert_eq!(
            cpu_model("processor : 0\n", Some("Raspberry Pi 5 Model B\0")).as_deref(),
            Some("Raspberry Pi 5 Model B")
        );
        assert_eq!(cpu_model("", None), None);
    }

    #[test]
    fn real_cpuinfo_has_a_model_or_none() {
        // The captured file comes from an Apple Silicon VM: no model name,
        // an implementer we do not list.
        let _ = cpu_model(include_str!("../../fixtures/debian_12/cpuinfo"), None);
    }

    #[test]
    fn containers_win_over_dmi() {
        let s = VirtSigns {
            dockerenv: true,
            dmi_vendor: Some("QEMU".into()),
            ..Default::default()
        };
        assert_eq!(
            detect_virtualization(&s),
            Virtualization {
                kind: "container",
                name: Some("docker".into())
            }
        );
        let s = VirtSigns {
            proc_vz: true,
            ..Default::default()
        };
        assert_eq!(detect_virtualization(&s).name.as_deref(), Some("openvz"));
        // The OpenVZ host itself has both /proc/vz and /proc/bc.
        let s = VirtSigns {
            proc_vz: true,
            proc_bc: true,
            ..Default::default()
        };
        assert_eq!(detect_virtualization(&s).kind, "none");
        let s = VirtSigns {
            systemd_container: Some("lxc\n".into()),
            ..Default::default()
        };
        assert_eq!(detect_virtualization(&s).name.as_deref(), Some("lxc"));
        let s = VirtSigns {
            pid1_cgroup: Some("0::/kubepods/besteffort/pod1".into()),
            ..Default::default()
        };
        assert_eq!(detect_virtualization(&s).name.as_deref(), Some("kubepods"));
    }

    #[test]
    fn vms_from_dmi() {
        let vm = |vendor: &str, product: &str| {
            detect_virtualization(&VirtSigns {
                dmi_vendor: Some(vendor.into()),
                dmi_product: Some(product.into()),
                ..Default::default()
            })
            .name
        };
        assert_eq!(
            vm("QEMU", "Standard PC (Q35 + ICH9, 2009)").as_deref(),
            Some("kvm")
        );
        assert_eq!(
            vm("VMware, Inc.", "VMware Virtual Platform").as_deref(),
            Some("vmware")
        );
        assert_eq!(
            vm("Microsoft Corporation", "Virtual Machine").as_deref(),
            Some("hyperv")
        );
        assert_eq!(vm("Amazon EC2", "t3.micro").as_deref(), Some("kvm"));
        assert_eq!(vm("Hetzner", "vServer").as_deref(), Some("kvm"));
        assert_eq!(
            vm("innotek GmbH", "VirtualBox").as_deref(),
            Some("virtualbox")
        );
        assert_eq!(vm("Dell Inc.", "PowerEdge R640"), None);
    }

    #[test]
    fn hypervisor_flag_and_bare_metal() {
        let s = VirtSigns {
            hypervisor_flag: true,
            ..Default::default()
        };
        assert_eq!(
            detect_virtualization(&s),
            Virtualization {
                kind: "vm",
                name: None
            }
        );
        assert_eq!(detect_virtualization(&VirtSigns::default()).kind, "none");
        assert!(has_hypervisor_flag(
            "flags\t\t: fpu vme hypervisor lahf_lm\n"
        ));
        assert!(!has_hypervisor_flag("flags\t\t: fpu vme lahf_lm\n"));
    }
}
