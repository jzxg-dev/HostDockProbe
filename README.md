# hostdock-probe

A small, read-only metrics probe for [HostDock](https://hostdock.dev). It runs on a Linux server and reports CPU, memory, disk, network and temperature to the HostDock probe service every 10 seconds. In the HostDock app you see live numbers and history, and get an email when a server goes offline or crosses a threshold.

The probe is open source so you can check what runs on your server. It **cannot run commands, change files or update itself**, does not listen on any port, does not need root, and talks to one fixed address. See [docs/SECURITY-MODEL.md](docs/SECURITY-MODEL.md) for each claim and how to verify it.

> **Status:** early. 0.1.1 is the first release; see [CHANGELOG.md](CHANGELOG.md).

## What it collects

| | |
|---|---|
| CPU | usage, iowait, steal |
| Load | 1, 5 and 15 minute averages; process count |
| Memory | used, available, cached, swap |
| Disk space | per mount point: size, used, available, inodes |
| Disk I/O | total read and write rate |
| Network | total receive and send rate, bytes per minute |
| Temperature | when the server has sensors |
| System | kernel, distribution, CPU model, cores, virtualization, uptime |

It does **not** collect process names or command lines, environment variables, users, logs, file contents, host names or IP addresses. Exactly where each number comes from: [docs/METRICS.md](docs/METRICS.md). Exactly what is sent: [docs/PROTOCOL.md](docs/PROTOCOL.md), or run `hostdock-probe once` to print a report without sending it.

## Requirements

- Linux on x86_64 or aarch64 (kernel 3.10 or later)
- Outbound HTTPS to `probe.hostdock.dev`

## Install

On the server (as root):

```
curl -fsSL https://github.com/jzxg-dev/HostDockProbe/releases/latest/download/install.sh | sudo sh
```

It asks for the token from "Add probe" in the HostDock app (input hidden), so the token stays out of your shell history and the process list. `--token hdp_...` also works but leaves the token in your history; `--token-stdin` reads it from standard input when you run a downloaded copy. Prefer to read the script first? Download [`install.sh`](dist/install.sh), read it, then run `sudo sh install.sh`. It:

1. downloads the binary for your CPU from the GitHub release and checks it against `SHA256SUMS`
2. creates the unprivileged system user and group `hostdock-probe`
3. installs `/usr/local/bin/hostdock-probe` and writes `/etc/hostdock-probe/config.toml`, owned by root and readable only by the `hostdock-probe` group (mode 640), so the probe cannot change its own config
4. installs and starts a hardened systemd unit ([`dist/hostdock-probe.service`](dist/hostdock-probe.service)) or, on Alpine, an OpenRC service

Running it again upgrades the binary and keeps the config. [`uninstall.sh`](dist/uninstall.sh) removes everything, including the user.

Tested on Debian 12, Ubuntu 24.04, Rocky Linux 9 and Alpine 3.20.

## Usage

```
hostdock-probe run       # run as a service
hostdock-probe once      # print one report as JSON, send nothing
hostdock-probe check     # check the config and whether the token is accepted
hostdock-probe version
```

Configuration lives in `/etc/hostdock-probe/config.toml`:

```toml
token = "hdp_..."            # from "Add probe" in the HostDock app

[collect]
temperature = true           # default
exclude_mounts = ["/mnt/*"]  # exact paths, or a prefix ending in *
exclude_interfaces = []      # same, for network interfaces
```

The token can also be given in `HOSTDOCK_PROBE_TOKEN`, and the config path in `HOSTDOCK_PROBE_CONFIG` or `--config`.

## Building from source

```
cargo build --release
```

The binary is `target/release/hostdock-probe`. It is a single file with no runtime dependencies besides libc; TLS uses rustls with bundled root certificates.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
