#!/bin/bash
set -euo pipefail
source "$(dirname "$0")/capture_session.sh"
runtime=$(mktemp -d /dev/shm/usd-session-test.XXXXXX)
waiter=
trap 'capture_session_stop "$runtime/session.pid" "$waiter"; rm -rf -- "$runtime"' EXIT
capture_session_run "$runtime/session.pid" bash -c 'trap "" TERM; sleep 600 & wait' & waiter=$!
for ((i=0; i<100; ++i)); do [[ ! -s "$runtime/session.pid" ]] || break; sleep .01; done
read -r leader < "$runtime/session.pid"
[[ "$leader" != "$waiter" ]]
kill -0 -- "-$leader"
capture_session_stop "$runtime/session.pid" "$waiter"
waiter=
if ps -eo pgid=,stat= | awk -v group="$leader" '$1 == group && $2 !~ /^Z/ {found=1} END {exit !found}'; then
    echo 'capture session retained running processes' >&2
    exit 1
fi
rm "$runtime/session.pid"
capture_session_run "$runtime/session.pid" bash -c 'exit 7' & waiter=$!
status=0
wait "$waiter" || status=$?
waiter=
[[ "$status" == 7 ]]
capture_session_run "$runtime/missing/session.pid" touch "$runtime/unsafe-launch" & waiter=$!
status=0
wait "$waiter" || status=$?
waiter=
[[ "$status" != 0 ]]
[[ ! -e "$runtime/unsafe-launch" ]]
echo 'CAPTURE_SESSION_OK forked leader, forced cleanup, exit status, failed handshake'
