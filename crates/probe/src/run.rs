//! Running as a service: sample every 5 seconds, measure disk space every
//! 30 seconds, report every 10 seconds (or whatever the service asks).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::agg::{Minute, MinuteAcc, minute_of};
use crate::collect::disks::{Disk, DiskMeter};
use crate::collect::system::{self, SystemInfo};
use crate::collect::{Collector, Options, Sample, unix_now};
use crate::config::Config;
use crate::log;
use crate::report::{self, Client, MAX_MINUTES, Outcome, PROTOCOL, ProbeInfo, Report, Snapshot};

pub const SAMPLE_EVERY: Duration = Duration::from_secs(5);
const DISKS_EVERY: Duration = Duration::from_secs(30);
const SYSTEM_EVERY: Duration = Duration::from_secs(3600);
const DEFAULT_INTERVAL: u64 = 10;
const BACKOFF: [u64; 5] = [1, 2, 5, 10, 30];
const UNAUTHORIZED_RETRY: u64 = 3600;
const CLOCK_SKEW_WARN: u64 = 60;

#[derive(Default)]
struct Shared {
    last: Option<Sample>,
    disks: Vec<Disk>,
    acc: Option<MinuteAcc>,
    minutes: VecDeque<Minute>,
    system: Option<SystemInfo>,
    /// Send `system` with the next report.
    system_due: bool,
}

pub fn read_mounts() -> String {
    std::fs::read_to_string("/proc/self/mounts").unwrap_or_default()
}

/// Feed a sample into the current minute; when the minute has changed,
/// finish the previous one and queue it, dropping the oldest beyond
/// [`MAX_MINUTES`].
fn add_sample(state: &mut Shared, s: Sample) {
    let minute = minute_of(s.at);
    if let Some(acc) = &state.acc
        && acc.minute() != minute
    {
        if !acc.is_empty() {
            let finished = acc.finish(state.disks.clone());
            state.minutes.push_back(finished);
            while state.minutes.len() > MAX_MINUTES {
                state.minutes.pop_front();
            }
        }
        state.acc = None;
    }
    state
        .acc
        .get_or_insert_with(|| MinuteAcc::new(minute))
        .add(&s);
    state.last = Some(s);
}

/// Drop the minutes a report carried. By value, not by count: the oldest
/// may have been dropped and new ones queued while the request was out.
fn drop_sent(state: &mut Shared, sent: &[Minute]) {
    if let Some(last) = sent.iter().map(|m| m.minute).max() {
        state.minutes.retain(|m| m.minute > last);
    }
}

fn build_report(state: &Shared) -> Report {
    let now = unix_now();
    let uptime = state.last.as_ref().and_then(|s| s.uptime);
    Report {
        protocol: PROTOCOL,
        probe: ProbeInfo::current(),
        sent_at: now,
        boot_time: report::boot_time(now, uptime),
        system: if state.system_due {
            state.system.clone()
        } else {
            None
        },
        snapshot: state
            .last
            .as_ref()
            .map(|s| Snapshot::new(s, state.disks.clone())),
        minutes: state.minutes.iter().cloned().collect(),
    }
}

fn reporter(shared: Arc<Mutex<Shared>>, client: Client) {
    let mut wait = Duration::from_secs(2); // let the first rates come in
    let mut failures = 0usize;
    let mut unauthorized = false;
    let mut connected = false;
    loop {
        thread::sleep(wait);
        let report = build_report(&shared.lock().unwrap());
        let outcome = client.send(&report);
        let mut state = shared.lock().unwrap();
        match outcome {
            Outcome::Accepted {
                interval,
                server_time,
            } => {
                drop_sent(&mut state, &report.minutes);
                if report.system.is_some() {
                    state.system_due = false;
                }
                drop(state);
                let secs = interval.unwrap_or(DEFAULT_INTERVAL).clamp(5, 300);
                if !connected || failures > 0 || unauthorized {
                    log::info(&format!("reporting to {} every {secs} s", report::ENDPOINT));
                }
                if let Some(t) = server_time
                    && t.abs_diff(report.sent_at) > CLOCK_SKEW_WARN
                {
                    log::warn(&format!(
                        "this server's clock is {} s off; check NTP",
                        t.abs_diff(report.sent_at)
                    ));
                }
                connected = true;
                failures = 0;
                unauthorized = false;
                wait = Duration::from_secs(secs);
            }
            Outcome::Unauthorized => {
                drop(state);
                if !unauthorized {
                    log::error(
                        "the probe service rejected the token (probe deleted or token rotated). \
                         Reinstall with the command from the HostDock app. Retrying every hour.",
                    );
                }
                unauthorized = true;
                wait = Duration::from_secs(UNAUTHORIZED_RETRY);
            }
            Outcome::RateLimited(after) => {
                drop(state);
                let secs = after.map_or(BACKOFF[failures.min(BACKOFF.len() - 1)], |s| {
                    s.clamp(5, 600)
                });
                log::warn(&format!("rate limited; waiting {secs} s"));
                failures += 1;
                wait = Duration::from_secs(secs);
            }
            Outcome::Rejected(status) => {
                drop_sent(&mut state, &report.minutes);
                drop(state);
                log::error(&format!(
                    "the probe service refused a report (HTTP {status}); dropped {} minute(s)",
                    report.minutes.len()
                ));
                wait = Duration::from_secs(DEFAULT_INTERVAL);
            }
            Outcome::Failed(reason) => {
                let queued = state.minutes.len();
                drop(state);
                if failures.is_multiple_of(20) {
                    log::warn(&format!(
                        "could not reach the probe service: {reason}; {queued} minute(s) queued"
                    ));
                }
                wait = Duration::from_secs(BACKOFF[failures.min(BACKOFF.len() - 1)]);
                failures += 1;
            }
        }
    }
}

fn disk_loop(shared: Arc<Mutex<Shared>>, exclude: Vec<String>) {
    let meter = DiskMeter::default();
    loop {
        let disks = meter.measure(&read_mounts(), &exclude);
        shared.lock().unwrap().disks = disks;
        thread::sleep(DISKS_EVERY);
    }
}

pub fn run(config: Config, token: String) -> ! {
    log::info(&format!("hostdock-probe {} starting", report::VERSION));
    let shared = Arc::new(Mutex::new(Shared::default()));
    let opts = Options {
        temperature: config.collect.temperature,
        exclude_interfaces: config.collect.exclude_interfaces.clone(),
    };

    // Disk space first, so the first report already has it.
    {
        let exclude = config.collect.exclude_mounts.clone();
        shared.lock().unwrap().disks = DiskMeter::default().measure(&read_mounts(), &exclude);
        let s = Arc::clone(&shared);
        thread::Builder::new()
            .name("disks".into())
            .spawn(move || disk_loop(s, exclude))
            .expect("start disk thread");
    }
    {
        let s = Arc::clone(&shared);
        let client = Client::new(token);
        thread::Builder::new()
            .name("report".into())
            .spawn(move || reporter(s, client))
            .expect("start report thread");
    }

    let mut collector = Collector::new(opts);
    let mut next = Instant::now();
    let mut system_checked: Option<Instant> = None;
    loop {
        let sample = collector.sample();
        let now = Instant::now();
        let system_info = if system_checked.is_none_or(|t| now.duration_since(t) >= SYSTEM_EVERY) {
            system_checked = Some(now);
            Some(system::collect(
                sample.cores,
                sample.memory.map(|m| m.total),
            ))
        } else {
            None
        };
        {
            let mut state = shared.lock().unwrap();
            if let Some(info) = system_info {
                // Sent at start, when it changes, and once an hour.
                state.system = Some(info);
                state.system_due = true;
            }
            add_sample(&mut state, sample);
        }
        next += SAMPLE_EVERY;
        let now = Instant::now();
        if next <= now {
            // We fell behind (suspended VM, very slow /proc): start afresh.
            next = now + SAMPLE_EVERY;
        }
        thread::sleep(next - now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at: u64) -> Sample {
        Sample {
            at,
            ..Default::default()
        }
    }

    #[test]
    fn minutes_roll_over_and_queue() {
        let mut st = Shared::default();
        for at in [60, 65, 70, 119, 120, 125, 185] {
            add_sample(&mut st, sample(at));
        }
        let queued: Vec<(u64, u32)> = st.minutes.iter().map(|m| (m.minute, m.samples)).collect();
        assert_eq!(queued, [(60, 4), (120, 2)]);
        assert_eq!(st.acc.as_ref().unwrap().minute(), 180);
    }

    #[test]
    fn queue_is_capped() {
        let mut st = Shared::default();
        for i in 0..(MAX_MINUTES as u64 + 10) {
            add_sample(&mut st, sample(i * 60));
        }
        assert_eq!(st.minutes.len(), MAX_MINUTES);
        assert_eq!(st.minutes.front().unwrap().minute, 9 * 60);
    }

    #[test]
    fn drop_sent_keeps_newer_minutes() {
        let mut st = Shared::default();
        for at in [0, 60, 120, 180] {
            add_sample(&mut st, sample(at));
        }
        let sent: Vec<Minute> = st.minutes.iter().take(2).cloned().collect();
        add_sample(&mut st, sample(240));
        drop_sent(&mut st, &sent);
        let left: Vec<u64> = st.minutes.iter().map(|m| m.minute).collect();
        assert_eq!(left, [120, 180]);
    }

    #[test]
    fn system_only_when_due() {
        let mut st = Shared {
            system: Some(system::collect(Some(2), Some(1))),
            ..Default::default()
        };
        assert!(build_report(&st).system.is_none());
        st.system_due = true;
        assert!(build_report(&st).system.is_some());
    }
}
