#!/bin/sh
# install.sh carries copies of the systemd unit and the OpenRC script so that it
# is a single file. This checks that the copies match the files next to it.
set -eu
cd "$(dirname "$0")"
ok=0
check() {
  awk "/<<'$1'/{f=1;next} /^$1\$/{f=0} f" install.sh | diff -u "$2" - || { echo "install.sh's copy of $2 differs" >&2; ok=1; }
}
check UNIT hostdock-probe.service
check OPENRC hostdock-probe.openrc
exit $ok
