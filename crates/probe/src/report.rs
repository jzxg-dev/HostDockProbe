//! The report sent to the probe service, and sending it. Field for field
//! this matches docs/PROTOCOL.md; change that document first.

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::time::Duration;

use crate::agg::Minute;
use crate::collect::counters::{IoRates, NetRates};
use crate::collect::cpu::CpuUsage;
use crate::collect::disks::Disk;
use crate::collect::load::Load;
use crate::collect::memory::Memory;
use crate::collect::system::SystemInfo;
use crate::collect::temp::Temperature;
use crate::collect::{Procs, Sample};

/// Where reports go. Fixed at build time and deliberately not
/// configurable. Development builds set `HOSTDOCK_PROBE_ENDPOINT`.
pub const ENDPOINT: &str = match option_env!("HOSTDOCK_PROBE_ENDPOINT") {
    Some(url) => url,
    None => "https://probe.hostdock.dev",
};

pub const PROTOCOL: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MAX_MINUTES: usize = 60;
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
pub struct ProbeInfo {
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<&'static str>,
}

impl ProbeInfo {
    pub fn current() -> Self {
        ProbeInfo {
            version: VERSION,
            commit: option_env!("HOSTDOCK_PROBE_COMMIT"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<Load>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procs: Option<Procs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    pub disks: Vec<Disk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_io: Option<IoRates>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetRates>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Temperature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime: Option<u64>,
}

impl Snapshot {
    pub fn new(s: &Sample, disks: Vec<Disk>) -> Self {
        Snapshot {
            at: s.at,
            cpu: s.cpu.map(round_cpu),
            load: s.load,
            procs: s.procs,
            memory: s.memory,
            disks,
            disk_io: s.disk_io,
            network: s.network,
            temperature: s.temperature.clone(),
            uptime: s.uptime.map(|u| u as u64),
        }
    }
}

fn round_cpu(c: CpuUsage) -> CpuUsage {
    let r = |v: f64| (v * 100.0).round() / 100.0;
    CpuUsage {
        usage: r(c.usage),
        iowait: r(c.iowait),
        steal: r(c.steal),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub protocol: u32,
    pub probe: ProbeInfo,
    pub sent_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
    pub minutes: Vec<Minute>,
}

pub fn boot_time(now: u64, uptime: Option<f64>) -> Option<u64> {
    uptime.map(|u| now.saturating_sub(u as u64))
}

/// The only part of the service's answer the probe reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportResponse {
    pub interval: Option<u64>,
    pub server_time: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Accepted {
        interval: Option<u64>,
        server_time: Option<u64>,
    },
    /// 401: the token is not (or no longer) valid.
    Unauthorized,
    /// 429: slow down, optionally for this many seconds.
    RateLimited(Option<u64>),
    /// 400, 413 or 422: the report itself was refused; do not send it again.
    Rejected(u16),
    /// Anything else (5xx, 3xx, 403, 404, a network error): keep the minutes
    /// and try again later. A 403 or 404 usually means something in front of
    /// the service is misconfigured, not that the report is bad.
    Failed(String),
}

pub fn classify(status: u16, retry_after: Option<&str>, body: &str) -> Outcome {
    match status {
        200..=299 => {
            let r: ReportResponse = serde_json::from_str(body).unwrap_or_default();
            Outcome::Accepted {
                interval: r.interval,
                server_time: r.server_time,
            }
        }
        401 => Outcome::Unauthorized,
        429 => Outcome::RateLimited(retry_after.and_then(|v| v.trim().parse().ok())),
        400 | 413 | 422 => Outcome::Rejected(status),
        _ => Outcome::Failed(format!("HTTP {status}")),
    }
}

pub fn gzip_json<T: Serialize>(value: &T) -> Vec<u8> {
    let json = serde_json::to_vec(value).expect("report serialises");
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&json).expect("writing to a Vec");
    enc.finish().expect("writing to a Vec")
}

pub struct Client {
    agent: ureq::Agent,
    token: String,
}

impl Client {
    pub fn new(token: String) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            // The probe talks to ENDPOINT and nowhere else: a redirect is not
            // followed (it comes back as a 3xx and is retried like a 5xx).
            .max_redirects(0)
            .user_agent(format!("hostdock-probe/{VERSION}"))
            .build()
            .into();
        Client { agent, token }
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    pub fn send(&self, report: &Report) -> Outcome {
        let body = gzip_json(report);
        let result = self
            .agent
            .post(format!("{ENDPOINT}/v1/report"))
            .header("Authorization", self.auth())
            .header("Content-Type", "application/json")
            .header("Content-Encoding", "gzip")
            .send(&body[..]);
        match result {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let text = resp
                    .body_mut()
                    .with_config()
                    .limit(64 * 1024)
                    .read_to_string()
                    .unwrap_or_default();
                classify(status, retry_after.as_deref(), &text)
            }
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }

    /// `GET /v1/check`: is the token valid?
    pub fn check(&self) -> Result<bool, String> {
        let resp = self
            .agent
            .get(format!("{ENDPOINT}/v1/check"))
            .header("Authorization", self.auth())
            .call()
            .map_err(|e| e.to_string())?;
        match resp.status().as_u16() {
            200..=299 => Ok(true),
            401 => Ok(false),
            s => Err(format!("HTTP {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn reads_only_interval_and_server_time() {
        let body =
            r#"{"interval": 30, "server_time": 5, "run": "rm -rf /", "update": {"url": "x"}}"#;
        assert_eq!(
            classify(200, None, body),
            Outcome::Accepted {
                interval: Some(30),
                server_time: Some(5)
            }
        );
        assert_eq!(
            classify(200, None, "not json"),
            Outcome::Accepted {
                interval: None,
                server_time: None
            }
        );
    }

    #[test]
    fn classifies_statuses() {
        assert_eq!(classify(401, None, ""), Outcome::Unauthorized);
        assert_eq!(
            classify(429, Some("120"), ""),
            Outcome::RateLimited(Some(120))
        );
        assert_eq!(classify(429, None, ""), Outcome::RateLimited(None));
        assert_eq!(classify(400, None, ""), Outcome::Rejected(400));
        assert_eq!(classify(413, None, ""), Outcome::Rejected(413));
        assert!(matches!(classify(503, None, ""), Outcome::Failed(_)));
        // A CDN challenge, a wrong path or a redirect: keep the data and retry.
        for status in [301, 307, 308, 403, 404, 405] {
            assert!(
                matches!(classify(status, None, ""), Outcome::Failed(_)),
                "{status}"
            );
        }
    }

    #[test]
    fn report_shape() {
        let report = Report {
            protocol: PROTOCOL,
            probe: ProbeInfo {
                version: "0.1.0",
                commit: None,
            },
            sent_at: 100,
            boot_time: boot_time(100, Some(40.7)),
            system: None,
            snapshot: Some(Snapshot::new(
                &Sample {
                    at: 95,
                    ..Default::default()
                },
                vec![],
            )),
            minutes: vec![],
        };
        let gz = gzip_json(&report);
        let mut json = String::new();
        GzDecoder::new(&gz[..]).read_to_string(&mut json).unwrap();
        assert_eq!(
            json,
            r#"{"protocol":1,"probe":{"version":"0.1.0"},"sent_at":100,"boot_time":60,"snapshot":{"at":95,"disks":[]},"minutes":[]}"#
        );
    }
}
