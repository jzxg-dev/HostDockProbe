# Reporting protocol

This is the complete contract between `hostdock-probe` and the HostDock probe service. The probe sends exactly what is described here and acts on nothing else.

- **Endpoint**: `https://probe.hostdock.dev`, fixed at build time. It cannot be changed in the config file.
- **Transport**: HTTPS (rustls with bundled Mozilla root certificates). Outbound only; the probe never listens on a port.
- **Compatibility**: probes do not update themselves, so old versions stay installed for a long time. Fields are only ever added. Existing fields are never renamed, removed or given a new meaning, and the service accepts every protocol version that is still in use.

## Authentication

Every request carries the probe's token:

```
Authorization: Bearer hdp_<32 letters and digits>
```

The token is created in the HostDock app ("Add probe") and is shown once. It identifies exactly one probe and can only be used for the two requests below. Deleting the probe or rotating its token invalidates it immediately. The service stores only a hash of it.

## `POST /v1/report`

Sent every 10 seconds by default (the service may ask for a different interval, see the response).

```
Content-Type: application/json
Content-Encoding: gzip
User-Agent: hostdock-probe/<version>
```

The body is a gzip-compressed JSON object:

```json
{
  "protocol": 1,
  "probe": {"version": "0.1.0", "commit": "a3f9c21"},
  "sent_at": 1793750400,
  "boot_time": 1790000000,
  "system": { ... },
  "snapshot": { ... },
  "minutes": [ { ... } ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `protocol` | integer | Protocol version, currently `1` |
| `probe.version` | string | Probe version |
| `probe.commit` | string or absent | Git commit the binary was built from |
| `sent_at` | integer | Probe's clock when the report was sent, Unix seconds |
| `boot_time` | integer or absent | When the server booted, Unix seconds (`sent_at` minus uptime). A jump of more than 60 seconds means the server rebooted |
| `system` | object or absent | Sent on the first successful report after start, whenever it changes, and once an hour |
| `snapshot` | object or absent | The latest sample, for the live view |
| `minutes` | array | Finished one-minute aggregates not yet acknowledged, oldest first, at most 60 |

All times are Unix seconds. All sizes are bytes, all rates bytes per second, all percentages 0–100, temperatures degrees Celsius. A field that could not be read is absent (never zero), so the service can tell "0 %" from "unknown".

### `system`

```json
{
  "kernel": "6.8.0-45-generic",
  "os": {"id": "ubuntu", "name": "Ubuntu", "version_id": "24.04", "pretty_name": "Ubuntu 24.04.1 LTS"},
  "arch": "x86_64",
  "cpu_model": "AMD EPYC 7B13",
  "cpu_cores": 4,
  "memory_total": 8323002368,
  "virtualization": {"kind": "vm", "name": "kvm"}
}
```

- `virtualization.kind` is `none`, `vm` or `container`. `name` is absent when the kind is known but the product is not (`kvm`, `xen`, `vmware`, `hyperv`, `virtualbox`, `openvz`, `lxc`, `docker`, `podman`, …). Inside a container, `/proc` may show the host's numbers rather than the container's.
- Any field that cannot be determined is absent.

### `snapshot`

```json
{
  "at": 1793750395,
  "cpu": {"usage": 23.1, "iowait": 2.0, "steal": 1.0},
  "load": {"one": 0.82, "five": 0.95, "fifteen": 1.02},
  "procs": {"total": 214, "running": 2},
  "memory": {"total": 8323002368, "available": 4910000000, "used": 3413002368,
             "cached": 2254000000, "swap_total": 0, "swap_used": 0},
  "disks": [{"mount": "/", "fs": "ext4", "total": 171798691840, "used": 65712000000,
             "available": 97500000000, "inodes_total": 10485760, "inodes_used": 943718}],
  "disk_io": {"read_bps": 1258291, "write_bps": 3984588},
  "network": {"rx_bps": 2516582, "tx_bps": 624640},
  "temperature": {"celsius": 54.0, "sensors": [{"name": "coretemp Package id 0", "celsius": 54.0}]},
  "uptime": 3643200
}
```

- `cpu`, `disk_io` and `network` are rates and need two samples, so they are absent in the first five seconds after start.
- `disks`: at most 20 entries. `inodes_total` and `inodes_used` are absent on file systems that do not have a fixed inode count (btrfs, for example). Disk usage in percent is `used / (used + available)`, the same as `df`.
- `temperature` is absent when the server has no readable sensors. `celsius` is the representative value used for charts and alerts; `sensors` lists at most 8.

### `minutes`

```json
{
  "minute": 1793750340,
  "samples": 12,
  "cpu": {"usage": {"avg": 22.4, "max": 41.0}, "iowait": {"avg": 1.8, "max": 6.0}, "steal": {"avg": 0.9, "max": 2.1}},
  "load": {"one": {"avg": 0.8, "max": 1.1}, "five": {"avg": 0.95, "max": 0.97}, "fifteen": {"avg": 1.02, "max": 1.02}},
  "procs": {"total": {"avg": 213.5, "max": 216}, "running": {"avg": 1.6, "max": 4}},
  "memory": {"total": 8323002368, "used": {"avg": 3400000000, "max": 3500000000},
             "cached": {"avg": 2250000000, "max": 2260000000},
             "swap_total": 0, "swap_used": {"avg": 0, "max": 0}},
  "disks": [ ... same shape as in snapshot ... ],
  "disk_io": {"read_bps": {"avg": 1200000, "max": 9800000}, "write_bps": {"avg": 3900000, "max": 12000000}},
  "network": {"rx_bps": {"avg": 2500000, "max": 7100000}, "tx_bps": {"avg": 620000, "max": 1900000},
              "rx_bytes": 150000000, "tx_bytes": 37200000},
  "temperature": {"avg": 53.6, "max": 58.0}
}
```

- `minute` is the start of the minute on the probe's clock. The service treats `(probe, minute)` as unique and keeps the last one it receives, so resending is safe.
- `samples` is how many 5-second samples went into it (12 when nothing was missed).
- `{"avg", "max"}` is the average and the highest of the samples in that minute. Slowly changing values (`memory.total`, `memory.swap_total`, `disks`) are the last value of the minute.
- `network.rx_bytes` / `tx_bytes` are the bytes received and sent during that minute, summed from counter differences. Adding them up gives monthly traffic.
- A group is absent when none of the minute's samples had it.

### Response

`200 OK` with a JSON body:

```json
{"interval": 10, "server_time": 1793750401}
```

| Field | What the probe does with it |
|---|---|
| `interval` | Seconds until the next report. Values outside 5–300 are clamped |
| `server_time` | Compared with the probe's clock; a difference over 60 seconds is logged as a warning |

The probe then drops every minute it sent. **No other field in the response is read.**

Other statuses:

| Status | What the probe does |
|---|---|
| `401` | The token is invalid (deleted probe, rotated token). Logs an error, keeps sampling, retries once an hour |
| `429` | Waits `Retry-After` seconds (clamped to 5–600), or backs off |
| `400`, `413`, `422` | The report itself was rejected. Logs it and drops those minutes, so one bad report cannot block the queue |
| anything else (`5xx`, `3xx`, `403`, `404`, …), network error | Keeps the minutes and retries after 1, 2, 5, 10, then every 30 seconds |

Redirects are never followed: the probe only talks to the address built into it, and a `3xx` is treated like a `5xx`.

## `GET /v1/check`

Used by `hostdock-probe check` to test the token without sending data. `200` means the token is valid; `401` means it is not. The body is ignored.
