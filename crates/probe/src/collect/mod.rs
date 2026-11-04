//! Reading the metrics. Each submodule parses one kind of file with pure
//! functions; [`Collector`] does the reading and keeps the previous
//! counters needed for rates.

pub mod counters;
pub mod cpu;
pub mod disks;
pub mod load;
pub mod memory;
pub mod system;
pub mod temp;

use serde::Serialize;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use counters::{Counters, Interface, IoRates, NetRates};

const SENSOR_RESCAN: Duration = Duration::from_secs(3600);

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Procs {
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<u64>,
}

/// One 5-second sample. Anything that could not be read is `None`.
#[derive(Debug, Clone, Default)]
pub struct Sample {
    /// Wall clock, Unix seconds.
    pub at: u64,
    pub cpu: Option<cpu::CpuUsage>,
    pub cores: Option<u32>,
    pub load: Option<load::Load>,
    pub procs: Option<Procs>,
    pub memory: Option<memory::Memory>,
    pub disk_io: Option<IoRates>,
    pub network: Option<NetRates>,
    /// Bytes received and sent since the previous sample.
    pub net_bytes: Option<(u64, u64)>,
    pub temperature: Option<temp::Temperature>,
    pub uptime: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub temperature: bool,
    pub exclude_interfaces: Vec<String>,
}

pub struct Collector {
    opts: Options,
    prev_cpu: Option<cpu::CpuTimes>,
    prev_disk: Option<(Instant, Counters)>,
    prev_net: Option<(Instant, Counters)>,
    sensors: Vec<temp::SensorFile>,
    sensors_scanned: Option<Instant>,
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn count_processes() -> Option<u64> {
    let entries = fs::read_dir("/proc").ok()?;
    Some(
        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().is_some_and(load::is_pid))
            .count() as u64,
    )
}

fn counted_disks(stats: &Counters) -> Vec<String> {
    stats
        .keys()
        .filter(|name| {
            counters::is_counted_disk(
                name,
                Path::new(&format!("/sys/block/{name}/device")).exists(),
            )
        })
        .cloned()
        .collect()
}

fn interfaces(stats: &Counters) -> Vec<Interface> {
    let mut list: Vec<Interface> = stats
        .keys()
        .map(|name| Interface {
            name: name.clone(),
            has_device: Path::new(&format!("/sys/class/net/{name}/device")).exists(),
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

impl Collector {
    pub fn new(opts: Options) -> Self {
        Collector {
            opts,
            prev_cpu: None,
            prev_disk: None,
            prev_net: None,
            sensors: Vec::new(),
            sensors_scanned: None,
        }
    }

    pub fn sample(&mut self) -> Sample {
        let now = Instant::now();
        let mut s = Sample {
            at: unix_now(),
            ..Default::default()
        };

        if let Some(stat) = read("/proc/stat").map(|t| cpu::parse_stat(&t)) {
            if let (Some(prev), Some(cur)) = (&self.prev_cpu, &stat.cpu) {
                s.cpu = cpu::usage(prev, cur);
            }
            if stat.cpu.is_some() {
                self.prev_cpu = stat.cpu;
            }
            s.cores = (stat.cores > 0).then_some(stat.cores);
            if let Some(total) = count_processes() {
                s.procs = Some(Procs {
                    total,
                    running: stat.procs_running,
                });
            }
        }
        s.load = read("/proc/loadavg").and_then(|t| load::parse_loadavg(&t));
        s.uptime = read("/proc/uptime").and_then(|t| load::parse_uptime(&t));
        s.memory = read("/proc/meminfo").and_then(|t| memory::parse_meminfo(&t));

        if let Some(stats) = read("/proc/diskstats").map(|t| counters::parse_diskstats(&t)) {
            if let Some((then, prev)) = &self.prev_disk {
                let (r, w) = counters::delta(prev, &stats, &counted_disks(&stats));
                let secs = now.duration_since(*then).as_secs_f64();
                s.disk_io = Some(IoRates {
                    read_bps: counters::per_second(r, secs),
                    write_bps: counters::per_second(w, secs),
                });
            }
            self.prev_disk = Some((now, stats));
        }

        if let Some(stats) = read("/proc/net/dev").map(|t| counters::parse_net_dev(&t)) {
            if let Some((then, prev)) = &self.prev_net {
                let names = counters::counted_interfaces(
                    &interfaces(&stats),
                    &self.opts.exclude_interfaces,
                );
                let (rx, tx) = counters::delta(prev, &stats, &names);
                let secs = now.duration_since(*then).as_secs_f64();
                s.network = Some(NetRates {
                    rx_bps: counters::per_second(rx, secs),
                    tx_bps: counters::per_second(tx, secs),
                });
                s.net_bytes = Some((rx, tx));
            }
            self.prev_net = Some((now, stats));
        }

        if self.opts.temperature {
            if self
                .sensors_scanned
                .is_none_or(|t| now.duration_since(t) >= SENSOR_RESCAN)
            {
                self.sensors = temp::scan();
                self.sensors_scanned = Some(now);
            }
            s.temperature = temp::read(&self.sensors);
        }
        s
    }
}
