#!/bin/sh
# Remove hostdock-probe completely: the service, the binary, the config, the log
# and the system user. https://github.com/jzxg-dev/HostDockProbe
#
#   curl -fsSL https://github.com/jzxg-dev/HostDockProbe/releases/latest/download/uninstall.sh | sudo sh
#
# The probe's history stays in HostDock until you delete the probe in the app.

set -eu

NAME="hostdock-probe"

[ "$(id -u)" = 0 ] || { echo "error: run as root (for example with sudo)" >&2; exit 1; }

if [ -f "/etc/systemd/system/$NAME.service" ] && command -v systemctl >/dev/null 2>&1; then
  systemctl disable --now "$NAME" >/dev/null 2>&1 || true
  rm -f "/etc/systemd/system/$NAME.service"
  systemctl daemon-reload >/dev/null 2>&1 || true
fi
if [ -f "/etc/init.d/$NAME" ]; then
  rc-service "$NAME" stop >/dev/null 2>&1 || true
  rc-update del "$NAME" default >/dev/null 2>&1 || true
  rm -f "/etc/init.d/$NAME"
fi
# Anything still running by hand.
if command -v pkill >/dev/null 2>&1; then
  pkill -x "$NAME" >/dev/null 2>&1 || true
fi

rm -f "/usr/local/bin/$NAME" "/var/log/$NAME.log"
rm -rf "/etc/${NAME:?}"

if id "$NAME" >/dev/null 2>&1; then
  if command -v userdel >/dev/null 2>&1; then
    userdel "$NAME" >/dev/null 2>&1 || true
  elif command -v deluser >/dev/null 2>&1; then
    deluser "$NAME" >/dev/null 2>&1 || true
  fi
fi
if grep -q "^$NAME:" /etc/group 2>/dev/null; then
  if command -v groupdel >/dev/null 2>&1; then
    groupdel "$NAME" >/dev/null 2>&1 || true
  elif command -v delgroup >/dev/null 2>&1; then
    delgroup "$NAME" >/dev/null 2>&1 || true
  fi
fi

echo "hostdock-probe has been removed. Delete the probe in the HostDock app to remove its history."
