//! Memory from `/proc/meminfo`.

use serde::Serialize;

/// All values in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub used: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let mut total = None;
    let mut free = 0;
    let mut available = None;
    let mut buffers = 0;
    let mut cached = 0;
    let mut reclaimable = 0;
    let mut swap_total = 0;
    let mut swap_free = 0;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = rest
            .split_ascii_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        let bytes = kb.saturating_mul(1024);
        match key {
            "MemTotal" => total = Some(bytes),
            "MemFree" => free = bytes,
            "MemAvailable" => available = Some(bytes),
            "Buffers" => buffers = bytes,
            "Cached" => cached = bytes,
            "SReclaimable" => reclaimable = bytes,
            "SwapTotal" => swap_total = bytes,
            "SwapFree" => swap_free = bytes,
            _ => {}
        }
    }
    let total = total?;
    // MemAvailable exists since Linux 3.14; before that, estimate it the way
    // `free` used to.
    let available = available.unwrap_or(free + buffers + cached).min(total);
    Some(Memory {
        total,
        available,
        used: total - available,
        cached: buffers + cached + reclaimable,
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_meminfo() {
        let m = parse_meminfo(include_str!("../../fixtures/debian_12/meminfo")).unwrap();
        assert_eq!(m.total, 16424800 * 1024);
        assert_eq!(m.available, 14777752 * 1024);
        assert_eq!(m.used, (16424800 - 14777752) * 1024);
        assert_eq!(m.cached, (136 + 639608 + 561296) * 1024);
    }

    #[test]
    fn estimates_available_on_old_kernels() {
        let text = "MemTotal: 1000 kB\nMemFree: 200 kB\nBuffers: 50 kB\nCached: 250 kB\n\
                    SwapTotal: 512 kB\nSwapFree: 500 kB\n";
        let m = parse_meminfo(text).unwrap();
        assert_eq!(m.available, 500 * 1024);
        assert_eq!(m.used, 500 * 1024);
        assert_eq!(m.swap_used, 12 * 1024);
    }

    #[test]
    fn missing_total_is_none() {
        assert_eq!(parse_meminfo("MemFree: 1 kB\n"), None);
    }

    #[test]
    fn available_never_exceeds_total() {
        // Seen in some OpenVZ containers.
        let m = parse_meminfo("MemTotal: 100 kB\nMemAvailable: 150 kB\n").unwrap();
        assert_eq!(m.used, 0);
    }
}
