//! CPU time from `/proc/stat`.

use serde::Serialize;

/// Cumulative CPU time from the aggregate `cpu` line, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    /// `guest` and `guest_nice` are already included in `user` and `nice`,
    /// so they are not added again.
    pub fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
    }
}

/// CPU usage between two samples, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CpuUsage {
    pub usage: f64,
    pub iowait: f64,
    pub steal: f64,
}

/// What `/proc/stat` tells us.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stat {
    pub cpu: Option<CpuTimes>,
    /// Number of `cpuN` lines.
    pub cores: u32,
    pub procs_running: Option<u64>,
}

pub fn parse_stat(text: &str) -> Stat {
    let mut stat = Stat::default();
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let Some(key) = fields.next() else { continue };
        if key == "cpu" {
            let values: Vec<u64> = fields.map_while(|f| f.parse().ok()).collect();
            // Kernels since 2.6.11 have at least the 8 fields we use.
            if values.len() >= 8 {
                stat.cpu = Some(CpuTimes {
                    user: values[0],
                    nice: values[1],
                    system: values[2],
                    idle: values[3],
                    iowait: values[4],
                    irq: values[5],
                    softirq: values[6],
                    steal: values[7],
                });
            }
        } else if key.len() > 3
            && key.starts_with("cpu")
            && key[3..].bytes().all(|b| b.is_ascii_digit())
        {
            stat.cores += 1;
        } else if key == "procs_running" {
            stat.procs_running = fields.next().and_then(|v| v.parse().ok());
        }
    }
    stat
}

/// Usage between `prev` and `now`. `None` when no time has passed or the
/// counters went backwards (which only happens across a reboot).
pub fn usage(prev: &CpuTimes, now: &CpuTimes) -> Option<CpuUsage> {
    let total = now.total().checked_sub(prev.total())?;
    if total == 0 {
        return None;
    }
    let idle = now.idle.saturating_sub(prev.idle);
    let iowait = now.iowait.saturating_sub(prev.iowait);
    let steal = now.steal.saturating_sub(prev.steal);
    let pct = |v: u64| (v as f64 / total as f64 * 100.0).clamp(0.0, 100.0);
    Some(CpuUsage {
        usage: pct(total.saturating_sub(idle + iowait)),
        iowait: pct(iowait),
        steal: pct(steal),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_stat() {
        let stat = parse_stat(include_str!("../../fixtures/debian_12/stat"));
        let cpu = stat.cpu.unwrap();
        assert_eq!(cpu.user, 786684);
        assert_eq!(cpu.idle, 495693694);
        assert_eq!(cpu.softirq, 148197);
        assert_eq!(stat.cores, 10);
        assert_eq!(stat.procs_running, Some(6));
    }

    #[test]
    fn parses_alpine_stat() {
        let stat = parse_stat(include_str!("../../fixtures/alpine_3_20/stat"));
        assert!(stat.cpu.is_some());
        assert!(stat.cores >= 1);
    }

    #[test]
    fn usage_counts_iowait_as_not_busy() {
        let prev = CpuTimes::default();
        let now = CpuTimes {
            user: 20,
            system: 10,
            idle: 50,
            iowait: 10,
            steal: 10,
            ..Default::default()
        };
        let u = usage(&prev, &now).unwrap();
        assert_eq!(u.usage, 40.0); // user + system + steal
        assert_eq!(u.iowait, 10.0);
        assert_eq!(u.steal, 10.0);
    }

    #[test]
    fn usage_is_none_without_progress_or_after_reset() {
        let a = CpuTimes {
            idle: 100,
            ..Default::default()
        };
        assert_eq!(usage(&a, &a), None);
        assert_eq!(usage(&a, &CpuTimes::default()), None);
    }

    #[test]
    fn short_cpu_line_is_ignored() {
        let stat = parse_stat("cpu 1 2 3 4\ncpu0 1 2 3 4\n");
        assert_eq!(stat.cpu, None);
        assert_eq!(stat.cores, 1);
    }
}
