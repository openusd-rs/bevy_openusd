#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/capture_session.sh"
runtime=$(mktemp -d /dev/shm/usd-session-test.XXXXXX)
waiter=
trap 'capture_session_stop "$runtime/session.pid" "$waiter"; rm -rf -- "$runtime"' EXIT
(
    export XDG_CONFIG_HOME=original-config XDG_DATA_HOME=original-data XDG_CACHE_HOME=original-cache
    export XDG_CURRENT_DESKTOP=original-desktop XDG_SESSION_DESKTOP=original-session
    export HYPRLAND_INSTANCE_SIGNATURE=original-instance HYPRLAND_CMD=original-command DISPLAY=original-display
    (
        capture_session_private_environment "$runtime"
        [[ "$XDG_CONFIG_HOME" == "$runtime/config" && "$XDG_DATA_HOME" == "$runtime/data" && "$XDG_CACHE_HOME" == "$runtime/cache" ]]
        [[ "$XDG_CURRENT_DESKTOP" == Weston && "$XDG_SESSION_DESKTOP" == weston ]]
        [[ ! -v HYPRLAND_INSTANCE_SIGNATURE && ! -v HYPRLAND_CMD && ! -v DISPLAY ]]
        [[ -d "$XDG_DATA_HOME" && -d "$XDG_CACHE_HOME" ]]
        [[ $(cat "$XDG_CONFIG_HOME/xdg-desktop-portal/portals.conf") == $'[preferred]\ndefault=gtk' ]]
    )
    [[ "$XDG_CONFIG_HOME" == original-config && "$XDG_DATA_HOME" == original-data && "$XDG_CACHE_HOME" == original-cache ]]
    [[ "$XDG_CURRENT_DESKTOP" == original-desktop && "$XDG_SESSION_DESKTOP" == original-session ]]
    [[ "$HYPRLAND_INSTANCE_SIGNATURE" == original-instance && "$HYPRLAND_CMD" == original-command && "$DISPLAY" == original-display ]]
)
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
echo 'CAPTURE_SESSION_OK private environment, forked leader, forced cleanup, exit status, failed handshake'
