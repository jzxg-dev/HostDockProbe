# Security model

`hostdock-probe` runs on your server around the clock, so it is built to be safe even if the HostDock probe service is compromised. This page lists what it can and cannot do, and how to check each claim yourself.

## What it cannot do

| Claim | How it is enforced | How to check |
|---|---|---|
| It cannot be told to run commands | There is no code that starts a process. The service's response is parsed into two numbers (`interval`, `server_time`); everything else is ignored | Search the source for `std::process` and `Command`: there are none. The response type is `ReportResponse` in `crates/probe/src/report.rs` |
| It cannot change files on your server | It never opens a file for writing. The systemd unit additionally mounts the whole file system read-only for it (`ProtectSystem=strict`) | Search for `File::create`, `OpenOptions`, `fs::write`: there are none |
| It cannot update itself | There is no update code. Upgrading means installing the new version yourself | Search for any download or exec code: there is none |
| It does not listen on any port | It only makes outbound HTTPS requests | `ss -ltnp` shows nothing for `hostdock-probe` |
| It only talks to one address | `https://probe.hostdock.dev`, fixed at build time; there is no config option to change it, and redirects are not followed | `ENDPOINT` and `max_redirects(0)` in `crates/probe/src/report.rs` |
| It does not need root | It runs as the unprivileged `hostdock-probe` user with no capabilities | `ps -o user= -C hostdock-probe` |
| It has no unsafe code | `#![forbid(unsafe_code)]` | `crates/probe/src/main.rs`, first line |

## What it reads

Only the files listed in [METRICS.md](METRICS.md), plus `statvfs` on mount points. It runs no external commands.

## What it deliberately does not collect

- Process lists, process names and command lines (command lines often contain passwords). It counts the numeric directories in `/proc` and never looks inside them.
- Environment variables, of any process.
- Users, logins, shell history, logs.
- File contents, file names, directory listings (other than counting `/proc` entries).
- Host name, IP addresses, MAC addresses, open ports, connections.
- Hardware serial numbers and other root-only DMI data.
- Docker containers and images.

The only free text that comes from your server rather than from the kernel or hardware is **mount point paths** (for example `/mnt/backups`). Exclude any you do not want sent with `exclude_mounts` in the config file.

## What the service learns

Everything in a report, as described in [PROTOCOL.md](PROTOCOL.md), stored unencrypted so it can draw charts and send alerts. The service also sees the IP address the report comes from (any HTTPS server does); it does not store it. Run `hostdock-probe once` to print a report exactly as it would be sent.

## The token

The token in `/etc/hostdock-probe/config.toml` (owned by root, readable only by root and the `hostdock-probe` group, so the probe can read it but not change it) lets whoever has it send reports for that one probe. It cannot be used to read data or to reach your HostDock account. It is never written to logs. If it leaks, rotate it in the HostDock app; the old one stops working immediately.

## Hardening under systemd

The unit in [`dist/hostdock-probe.service`](../dist/hostdock-probe.service) runs the probe with `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=read-only`, `PrivateTmp`, `PrivateDevices`, an empty `CapabilityBoundingSet`, `RestrictAddressFamilies=AF_INET AF_INET6`, `SystemCallFilter=@system-service`, `MemoryMax=32M` and `CPUQuota=5%`. `ProtectHome` is `read-only` rather than `yes` so that the size of a separate `/home` partition can still be read.

## Verifying a download

Release binaries are built by GitHub Actions from the tagged source ([`.github/workflows/release.yml`](../.github/workflows/release.yml)), never on a developer's machine. Each release has a `SHA256SUMS` file and a GitHub build provenance attestation that ties every file to the workflow run and the commit it was built from:

```
gh attestation verify hostdock-probe-x86_64-linux -R jzxg-dev/HostDockProbe
```

The install script refuses to install a binary whose SHA-256 does not match `SHA256SUMS`. To check by hand, download the binary and `SHA256SUMS` from the same release and run `sha256sum -c SHA256SUMS --ignore-missing`.

## Reporting a vulnerability

See [SECURITY.md](../SECURITY.md).
