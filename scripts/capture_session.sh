#!/bin/bash

capture_session_stop() {
    local pid_file=$1 waiter=${2:-} leader= attempt
    for ((attempt=0; attempt<100; ++attempt)); do
        [[ ! -s "$pid_file" ]] || break
        [[ -n "$waiter" ]] && kill -0 "$waiter" 2>/dev/null || break
        sleep .01
    done
    if [[ -s "$pid_file" ]]; then
        read -r leader < "$pid_file"
        if [[ "$leader" =~ ^[1-9][0-9]*$ && "$leader" -gt 1 ]]; then
            kill -TERM -- "-$leader" 2>/dev/null || true
            for ((attempt=0; attempt<50; ++attempt)); do
                kill -0 -- "-$leader" 2>/dev/null || break
                sleep .02
            done
            kill -KILL -- "-$leader" 2>/dev/null || true
        fi
    fi
    if [[ -n "$waiter" ]]; then wait "$waiter" 2>/dev/null || true; fi
}

capture_session_run() {
    local pid_file=$1
    shift
    exec setsid --fork --wait /bin/bash -c 'set -e; printf "%s\n" "$$" > "$1"; printf "CAPTURE_SESSION_LEADER=%s\n" "$$" >&2; shift; exec "$@"' capture-session "$pid_file" "$@"
}
