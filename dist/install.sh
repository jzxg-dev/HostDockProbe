#!/bin/sh
# Install or upgrade hostdock-probe. https://github.com/jzxg-dev/HostDockProbe
#
#   curl -fsSL https://github.com/jzxg-dev/HostDockProbe/releases/latest/download/install.sh | sudo sh
#
# It asks for the token (input hidden), so the token stays out of your shell
# history and the process list. Or download it, read it, then run: sudo sh install.sh
# Running it again upgrades the binary and keeps the existing config.
#
# What it does, and nothing else:
#   1. downloads the binary for this CPU from GitHub Releases and checks its SHA-256
#   2. creates the unprivileged system user "hostdock-probe"
#   3. installs /usr/local/bin/hostdock-probe
#   4. writes /etc/hostdock-probe/config.toml (owned by root, readable by that user's group only)
#   5. installs and starts a systemd unit or an OpenRC service
#
# Options:
#   (none)            ask for the token on the terminal (only on a first install)
#   --token-stdin     read the token from the first line of standard input
#                     (for a downloaded install.sh; not with curl | sh)
#   --token TOKEN     pass the token directly (it ends up in your shell history)
#   HOSTDOCK_PROBE_TOKEN=...  same, from the environment
#   --version X.Y.Z   install this version instead of the latest
#   --binary PATH     install this file instead of downloading (no checksum check)
#   --no-start        install but do not start the service
#
# Remove everything again with uninstall.sh from the same release.

set -eu

REPO="https://github.com/jzxg-dev/HostDockProbe"
NAME="hostdock-probe"
BIN="/usr/local/bin/$NAME"
CONF_DIR="/etc/$NAME"
CONF="$CONF_DIR/config.toml"
USER_NAME="$NAME"

TOKEN="${HOSTDOCK_PROBE_TOKEN:-}"
TOKEN_STDIN=0
VERSION=""
LOCAL_BINARY=""
START=1

say() { printf '%s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --token) [ $# -ge 2 ] || fail "--token needs a value"; TOKEN="$2"; shift 2 ;;
    --token-stdin) TOKEN_STDIN=1; shift ;;
    --version) [ $# -ge 2 ] || fail "--version needs a value"; VERSION="$2"; shift 2 ;;
    --binary) [ $# -ge 2 ] || fail "--binary needs a path"; LOCAL_BINARY="$2"; shift 2 ;;
    --no-start) START=0; shift ;;
    -h|--help) sed -n '2,/^# Remove everything/p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done

[ "$(id -u)" = 0 ] || fail "run as root (for example with sudo)"
[ "$(uname -s)" = Linux ] || fail "hostdock-probe only runs on Linux"

case "$(uname -m)" in
  x86_64|amd64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) fail "unsupported CPU: $(uname -m) (x86_64 and aarch64 are supported)" ;;
esac

# The token: hdp_ followed by letters and digits. Never printed.
if [ "$TOKEN_STDIN" = 1 ]; then
  IFS= read -r TOKEN || [ -n "$TOKEN" ] || fail "--token-stdin: nothing on standard input"
elif [ -z "$TOKEN" ] && [ ! -f "$CONF" ] && (: </dev/tty) 2>/dev/null; then
  # stdin is this script when piped from curl, so ask on the terminal instead.
  printf 'Paste the token from "Add probe" in the HostDock app (input hidden): ' >/dev/tty
  trap 'stty echo </dev/tty 2>/dev/null; exit 130' INT TERM
  stty -echo </dev/tty 2>/dev/null || true
  IFS= read -r TOKEN </dev/tty || TOKEN=""
  stty echo </dev/tty 2>/dev/null || true
  trap - INT TERM
  printf '\n' >/dev/tty
fi
TOKEN="$(printf '%s' "$TOKEN" | tr -d '[:space:]')"
if [ -n "$TOKEN" ]; then
  case "$TOKEN" in
    hdp_*) ;;
    *) fail "the token should start with hdp_; copy it again from the HostDock app" ;;
  esac
  case "${TOKEN#hdp_}" in
    *[!A-Za-z0-9]*|'') fail "the token contains unexpected characters; copy it again from the HostDock app" ;;
  esac
elif [ ! -f "$CONF" ]; then
  fail "no token: run this on a terminal to be asked for it, or use --token-stdin (the token is in \"Add probe\" in the HostDock app)"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    fail "neither curl nor wget is installed"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    fail "no sha256sum or shasum to check the download"
  fi
}

# 1. The binary.
if [ -n "$LOCAL_BINARY" ]; then
  [ -f "$LOCAL_BINARY" ] || fail "no such file: $LOCAL_BINARY"
  cp "$LOCAL_BINARY" "$TMP/$NAME"
else
  if [ -n "$VERSION" ]; then
    BASE="$REPO/releases/download/v${VERSION#v}"
  else
    BASE="$REPO/releases/latest/download"
  fi
  FILE="$NAME-$ARCH-linux"
  say "Downloading $FILE ..."
  fetch "$BASE/$FILE" "$TMP/$NAME" || fail "download failed: $BASE/$FILE"
  fetch "$BASE/SHA256SUMS" "$TMP/SHA256SUMS" || fail "download failed: $BASE/SHA256SUMS"
  WANT="$(grep " $FILE\$" "$TMP/SHA256SUMS" | cut -d' ' -f1)"
  [ -n "$WANT" ] || fail "$FILE is not listed in SHA256SUMS"
  GOT="$(sha256 "$TMP/$NAME")"
  [ "$WANT" = "$GOT" ] || fail "checksum mismatch for $FILE; not installing"
  say "Checksum OK."
fi
chmod 0755 "$TMP/$NAME"
"$TMP/$NAME" version >/dev/null 2>&1 || fail "the downloaded binary does not run on this server"

# 2. The user, and a group of the same name (it is what may read the config).
if ! id "$USER_NAME" >/dev/null 2>&1; then
  if command -v useradd >/dev/null 2>&1; then
    NOLOGIN="$(command -v nologin || echo /bin/false)"
    grep -q "^$USER_NAME:" /etc/group || groupadd --system "$USER_NAME"
    useradd --system --no-create-home --home-dir / --shell "$NOLOGIN" -g "$USER_NAME" "$USER_NAME"
  elif command -v adduser >/dev/null 2>&1; then
    # BusyBox (Alpine): adduser -S does not create a group of the same name.
    addgroup -S "$USER_NAME" 2>/dev/null || true
    adduser -S -D -H -h / -s /sbin/nologin -G "$USER_NAME" "$USER_NAME"
  else
    fail "cannot create the user $USER_NAME (no useradd or adduser)"
  fi
  say "Created the system user $USER_NAME."
fi

# 3. The binary goes in place; a running service keeps the old file until restarted.
install -m 0755 "$TMP/$NAME" "$BIN.new" 2>/dev/null || { cp "$TMP/$NAME" "$BIN.new"; chmod 0755 "$BIN.new"; }
mv -f "$BIN.new" "$BIN"
say "Installed $("$BIN" version | head -n 1)."

# 4. The config. Kept on upgrade unless a new token is given. The directory and
# the file belong to root and the probe's group may only read them, so the probe
# cannot change its own config.
mkdir -p "$CONF_DIR"
chown root:"$USER_NAME" "$CONF_DIR"
chmod 0750 "$CONF_DIR"
[ ! -L "$CONF" ] || fail "$CONF is a symbolic link; remove it and run this again"
if [ -n "$TOKEN" ]; then
  (
    umask 077
    if [ -f "$CONF" ]; then
      grep -v '^[[:space:]]*token[[:space:]]*=' "$CONF" > "$TMP/config.rest" || true
    else
      : > "$TMP/config.rest"
    fi
    { printf 'token = "%s"\n' "$TOKEN"; cat "$TMP/config.rest"; } > "$TMP/config.toml"
  )
  chown root:"$USER_NAME" "$TMP/config.toml"
  chmod 0640 "$TMP/config.toml"
  mv -f "$TMP/config.toml" "$CONF"
fi
chown root:"$USER_NAME" "$CONF"
chmod 0640 "$CONF"

# 5. The service.
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
  cat > "/etc/systemd/system/$NAME.service" <<'UNIT'
# systemd unit for hostdock-probe. See docs/SECURITY-MODEL.md.
[Unit]
Description=HostDock probe (read-only metrics)
Documentation=https://github.com/jzxg-dev/HostDockProbe
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hostdock-probe
Group=hostdock-probe
ExecStart=/usr/local/bin/hostdock-probe run --config /etc/hostdock-probe/config.toml
Restart=always
RestartSec=10

# The probe needs nothing beyond reading /proc, /sys and statvfs.
NoNewPrivileges=yes
CapabilityBoundingSet=
AmbientCapabilities=
ProtectSystem=strict
# read-only, not yes: the size of a separate /home partition must stay readable.
ProtectHome=read-only
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictAddressFamilies=AF_INET AF_INET6
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources
# Not set on purpose: ProtectProc=invisible and ProcSubset=pid would hide
# the /proc entries the probe counts and reads.
MemoryMax=32M
CPUQuota=5%

[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload
  systemctl enable "$NAME" >/dev/null 2>&1
  if [ "$START" = 1 ]; then
    systemctl restart "$NAME"
    say "Started: systemctl status $NAME, logs: journalctl -u $NAME"
  fi
elif command -v openrc-run >/dev/null 2>&1 || [ -x /sbin/openrc-run ]; then
  cat > "/etc/init.d/$NAME" <<'OPENRC'
#!/sbin/openrc-run
# OpenRC service for hostdock-probe (Alpine and other OpenRC systems).
name="hostdock-probe"
description="HostDock probe (read-only metrics)"
supervisor="supervise-daemon"
command="/usr/local/bin/hostdock-probe"
command_args="run --config /etc/hostdock-probe/config.toml"
command_user="hostdock-probe:hostdock-probe"
respawn_delay=10
output_log="/var/log/hostdock-probe.log"
error_log="/var/log/hostdock-probe.log"

depend() {
	need net
}

start_pre() {
	checkpath -f -m 0640 -o hostdock-probe:hostdock-probe /var/log/hostdock-probe.log
}
OPENRC
  chmod 0755 "/etc/init.d/$NAME"
  rc-update add "$NAME" default >/dev/null 2>&1 || true
  if [ "$START" = 1 ]; then
    rc-service "$NAME" restart >/dev/null 2>&1 || rc-service "$NAME" start
    say "Started: rc-service $NAME status, logs: /var/log/hostdock-probe.log"
  fi
else
  say "Neither systemd nor OpenRC was found. Run it yourself as the $USER_NAME user:"
  say "  $BIN run --config $CONF"
fi

# Is the token accepted? Not fatal: the service keeps retrying.
if "$BIN" check --config "$CONF" >/dev/null 2>&1; then
  say "The probe service accepted the token. The server shows up in HostDock within a few seconds."
else
  say "Could not confirm the token with the probe service yet. Check with: $BIN check --config $CONF"
fi
