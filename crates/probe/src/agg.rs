//! One-minute aggregates. See docs/PROTOCOL.md, "minutes".

use serde::Serialize;

use crate::collect::Sample;
use crate::collect::disks::Disk;

/// Average and highest of the samples in a minute.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Stat {
    pub avg: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    sum: f64,
    max: f64,
    n: u32,
}

impl Acc {
    fn add(&mut self, v: f64) {
        if self.n == 0 || v > self.max {
            self.max = v;
        }
        self.sum += v;
        self.n += 1;
    }
    fn get(&self) -> Option<Stat> {
        (self.n > 0).then(|| Stat {
            avg: round(self.sum / self.n as f64),
            max: round(self.max),
        })
    }
}

/// Two decimals is plenty and keeps reports small.
fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CpuMinute {
    pub usage: Stat,
    pub iowait: Stat,
    pub steal: Stat,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoadMinute {
    pub one: Stat,
    pub five: Stat,
    pub fifteen: Stat,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcsMinute {
    pub total: Stat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<Stat>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemoryMinute {
    pub total: u64,
    pub used: Stat,
    pub cached: Stat,
    pub swap_total: u64,
    pub swap_used: Stat,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiskIoMinute {
    pub read_bps: Stat,
    pub write_bps: Stat,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetworkMinute {
    pub rx_bps: Stat,
    pub tx_bps: Stat,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Minute {
    pub minute: u64,
    pub samples: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuMinute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<LoadMinute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procs: Option<ProcsMinute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryMinute>,
    pub disks: Vec<Disk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_io: Option<DiskIoMinute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkMinute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Stat>,
}

/// Collects the samples of one minute.
#[derive(Debug, Clone, Default)]
pub struct MinuteAcc {
    minute: u64,
    samples: u32,
    cpu: [Acc; 3],
    load: [Acc; 3],
    procs: Acc,
    running: Acc,
    mem_used: Acc,
    mem_cached: Acc,
    swap_used: Acc,
    mem_total: Option<u64>,
    swap_total: u64,
    io: [Acc; 2],
    net: [Acc; 2],
    net_bytes: (u64, u64),
    temp: Acc,
}

pub fn minute_of(at: u64) -> u64 {
    at - at % 60
}

impl MinuteAcc {
    pub fn new(minute: u64) -> Self {
        MinuteAcc {
            minute,
            ..Default::default()
        }
    }

    pub fn minute(&self) -> u64 {
        self.minute
    }

    pub fn is_empty(&self) -> bool {
        self.samples == 0
    }

    pub fn add(&mut self, s: &Sample) {
        self.samples += 1;
        if let Some(c) = s.cpu {
            self.cpu[0].add(c.usage);
            self.cpu[1].add(c.iowait);
            self.cpu[2].add(c.steal);
        }
        if let Some(l) = s.load {
            self.load[0].add(l.one);
            self.load[1].add(l.five);
            self.load[2].add(l.fifteen);
        }
        if let Some(p) = s.procs {
            self.procs.add(p.total as f64);
            if let Some(r) = p.running {
                self.running.add(r as f64);
            }
        }
        if let Some(m) = s.memory {
            self.mem_used.add(m.used as f64);
            self.mem_cached.add(m.cached as f64);
            self.swap_used.add(m.swap_used as f64);
            self.mem_total = Some(m.total);
            self.swap_total = m.swap_total;
        }
        if let Some(io) = s.disk_io {
            self.io[0].add(io.read_bps as f64);
            self.io[1].add(io.write_bps as f64);
        }
        if let Some(n) = s.network {
            self.net[0].add(n.rx_bps as f64);
            self.net[1].add(n.tx_bps as f64);
        }
        if let Some((rx, tx)) = s.net_bytes {
            self.net_bytes.0 = self.net_bytes.0.saturating_add(rx);
            self.net_bytes.1 = self.net_bytes.1.saturating_add(tx);
        }
        if let Some(t) = &s.temperature {
            self.temp.add(t.celsius);
        }
    }

    /// `disks` is the latest disk space reading; disk space changes slowly,
    /// so the minute keeps only the last value.
    pub fn finish(&self, disks: Vec<Disk>) -> Minute {
        let three = |a: &[Acc; 3]| Some((a[0].get()?, a[1].get()?, a[2].get()?));
        Minute {
            minute: self.minute,
            samples: self.samples,
            cpu: three(&self.cpu).map(|(usage, iowait, steal)| CpuMinute {
                usage,
                iowait,
                steal,
            }),
            load: three(&self.load).map(|(one, five, fifteen)| LoadMinute { one, five, fifteen }),
            procs: self.procs.get().map(|total| ProcsMinute {
                total,
                running: self.running.get(),
            }),
            memory: self.mem_total.and_then(|total| {
                Some(MemoryMinute {
                    total,
                    used: self.mem_used.get()?,
                    cached: self.mem_cached.get()?,
                    swap_total: self.swap_total,
                    swap_used: self.swap_used.get()?,
                })
            }),
            disks,
            disk_io: match (self.io[0].get(), self.io[1].get()) {
                (Some(read_bps), Some(write_bps)) => Some(DiskIoMinute {
                    read_bps,
                    write_bps,
                }),
                _ => None,
            },
            network: match (self.net[0].get(), self.net[1].get()) {
                (Some(rx_bps), Some(tx_bps)) => Some(NetworkMinute {
                    rx_bps,
                    tx_bps,
                    rx_bytes: self.net_bytes.0,
                    tx_bytes: self.net_bytes.1,
                }),
                _ => None,
            },
            temperature: self.temp.get(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::counters::NetRates;
    use crate::collect::cpu::CpuUsage;
    use crate::collect::memory::Memory;

    fn sample(cpu: Option<f64>, rx: u64, bytes: Option<u64>) -> Sample {
        Sample {
            at: 120,
            cpu: cpu.map(|c| CpuUsage {
                usage: c,
                iowait: 1.0,
                steal: 0.0,
            }),
            network: bytes.map(|_| NetRates {
                rx_bps: rx,
                tx_bps: 0,
            }),
            net_bytes: bytes.map(|b| (b, 0)),
            memory: Some(Memory {
                total: 1000,
                available: 600,
                used: 400,
                cached: 100,
                swap_total: 0,
                swap_used: 0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn averages_and_maxima() {
        let mut acc = MinuteAcc::new(120);
        acc.add(&sample(None, 0, None)); // first sample: no rates yet
        acc.add(&sample(Some(10.0), 100, Some(500)));
        acc.add(&sample(Some(30.0), 300, Some(1500)));
        let m = acc.finish(vec![]);
        assert_eq!(m.samples, 3);
        assert_eq!(
            m.cpu.unwrap().usage,
            Stat {
                avg: 20.0,
                max: 30.0
            }
        );
        let net = m.network.unwrap();
        assert_eq!(
            net.rx_bps,
            Stat {
                avg: 200.0,
                max: 300.0
            }
        );
        assert_eq!(net.rx_bytes, 2000);
        assert_eq!(
            m.memory.unwrap().used,
            Stat {
                avg: 400.0,
                max: 400.0
            }
        );
        assert!(m.temperature.is_none());
        assert!(m.load.is_none());
    }

    #[test]
    fn minute_boundaries() {
        assert_eq!(minute_of(1793750399), 1793750340);
        assert_eq!(minute_of(1793750400), 1793750400);
    }

    #[test]
    fn serialises_without_absent_groups() {
        let m = MinuteAcc::new(60).finish(vec![]);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#"{"minute":60,"samples":0,"disks":[]}"#);
    }
}
