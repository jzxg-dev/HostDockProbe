# Changelog

## 0.1.2 (unreleased)

- `install.sh` asks for a token on an upgrade too: paste a new one to replace the old token, or press Enter to keep it. Changing the token no longer means editing the config by hand.
- `install.sh` tells a rejected token apart from a probe service it cannot reach yet.
- `install.sh` no longer prints a "Broken pipe" panic while reading the installed version.

## 0.1.1 (2026-11-04)

The first release. (0.1.0 was tagged, but its release build failed and nothing was published.)

- Collects CPU (usage, iowait, steal), load and process count, memory and swap, disk space and inodes per mount point, total disk I/O, total network traffic, temperature when the server has sensors, and system information (kernel, distribution, CPU model, cores, virtualization, uptime). Only `/proc`, `/sys`, `/etc/os-release` and `statvfs` are read; no external commands are run.
- Samples every 5 seconds, aggregates per minute and reports to `https://probe.hostdock.dev` every 10 seconds over HTTPS (rustls with bundled root certificates). Up to 60 minutes are kept in memory while the service cannot be reached and sent once it is back.
- Reads only the report interval and the server time from the service's answer. There is no way for the service to run commands, change files or update the probe. Redirects are not followed.
- `hostdock-probe once` prints a report exactly as it would be sent; `hostdock-probe check` tests the config and the token.
- `install.sh` downloads the binary for the CPU (x86_64 or aarch64, static musl builds), checks it against `SHA256SUMS`, creates the `hostdock-probe` user and group, writes a root-owned config only that group can read, and starts a hardened systemd unit or an OpenRC service. It asks for the token on the terminal so the token stays out of shell history. `uninstall.sh` removes everything.
- Every release file carries a GitHub build provenance attestation.
