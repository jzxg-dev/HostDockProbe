#![forbid(unsafe_code)]
//! hostdock-probe: a read-only metrics probe for HostDock.
//! See docs/SECURITY-MODEL.md for what it can and cannot do.

mod agg;
mod collect;
mod config;
mod log;
mod report;
mod run;

use std::process::ExitCode;
use std::thread;

use collect::disks::DiskMeter;
use collect::{Collector, Options, system, unix_now};
use report::{Client, PROTOCOL, ProbeInfo, Report, Snapshot};

const USAGE: &str = "\
hostdock-probe: read-only metrics probe for HostDock

Usage: hostdock-probe <command> [--config <path>]

Commands:
  run       Run as a service: sample every 5 s and report to the probe service
  once      Take one sample and print the report exactly as it would be sent,
            without sending it
  check     Check the config file and whether the token is accepted
  version   Print the version

Options:
  --config <path>   Config file (default /etc/hostdock-probe/config.toml,
                    or HOSTDOCK_PROBE_CONFIG)

The token can also be given in HOSTDOCK_PROBE_TOKEN.
Source and documentation: https://github.com/XiangongCloud/HostDockProbe";

fn config_path(args: &[String]) -> Result<String, String> {
    let mut path = std::env::var("HOSTDOCK_PROBE_CONFIG")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| config::DEFAULT_PATH.to_string());
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                path = args.get(i + 1).cloned().ok_or("--config needs a path")?;
                i += 2;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(path)
}

/// `once`: what `run` would send, printed instead of sent.
fn once(cfg: &config::Config) -> String {
    let opts = Options {
        temperature: cfg.collect.temperature,
        exclude_interfaces: cfg.collect.exclude_interfaces.clone(),
    };
    let mut collector = Collector::new(opts);
    collector.sample();
    // Rates need two samples.
    thread::sleep(std::time::Duration::from_secs(1));
    let sample = collector.sample();
    let disks = DiskMeter::default().measure(&run::read_mounts(), &cfg.collect.exclude_mounts);
    let now = unix_now();
    let report = Report {
        protocol: PROTOCOL,
        probe: ProbeInfo::current(),
        sent_at: now,
        boot_time: report::boot_time(now, sample.uptime),
        system: Some(system::collect(
            sample.cores,
            sample.memory.map(|m| m.total),
        )),
        snapshot: Some(Snapshot::new(&sample, disks)),
        minutes: vec![],
    };
    serde_json::to_string_pretty(&report).expect("report serialises")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match command {
        "version" | "--version" | "-V" => {
            match option_env!("HOSTDOCK_PROBE_COMMIT") {
                Some(c) => println!("hostdock-probe {} ({c})", report::VERSION),
                None => println!("hostdock-probe {}", report::VERSION),
            }
            println!("protocol {PROTOCOL}, reports to {}", report::ENDPOINT);
            return ExitCode::SUCCESS;
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        "run" | "once" | "check" => {}
        other => {
            eprintln!("unknown command: {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }
    if !cfg!(target_os = "linux") && command != "check" {
        eprintln!("hostdock-probe collects metrics from /proc and only runs on Linux");
        return ExitCode::FAILURE;
    }
    let cfg = match config_path(&args[1..]).and_then(|p| config::load(&p)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    match command {
        "once" => {
            println!("{}", once(&cfg));
            ExitCode::SUCCESS
        }
        "check" => {
            let token = match cfg.token() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            println!("config: ok");
            match Client::new(token).check() {
                Ok(true) => {
                    println!("probe service ({}): token accepted", report::ENDPOINT);
                    ExitCode::SUCCESS
                }
                Ok(false) => {
                    println!(
                        "probe service ({}): token rejected; copy the install command again from the HostDock app",
                        report::ENDPOINT
                    );
                    ExitCode::FAILURE
                }
                Err(e) => {
                    println!("probe service ({}): cannot connect: {e}", report::ENDPOINT);
                    ExitCode::FAILURE
                }
            }
        }
        _ => match cfg.token() {
            Ok(token) => run::run(cfg, token),
            Err(e) => {
                log::error(&e);
                ExitCode::FAILURE
            }
        },
    }
}
