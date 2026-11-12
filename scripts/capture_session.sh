#!/usr/bin/env bash

capture_session_private_environment() {
    local runtime=$1
    export XDG_CONFIG_HOME="$runtime/config" XDG_DATA_HOME="$runtime/data" XDG_CACHE_HOME="$runtime/cache"
    export XDG_CURRENT_DESKTOP=Weston XDG_SESSION_DESKTOP=weston
    unset HYPRLAND_INSTANCE_SIGNATURE HYPRLAND_CMD DISPLAY
    mkdir -p "$XDG_CONFIG_HOME/xdg-desktop-portal" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
    printf '[preferred]\ndefault=gtk\n' > "$XDG_CONFIG_HOME/xdg-desktop-portal/portals.conf"
}

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
    exec setsid --fork --wait bash -c 'set -e; printf "%s\n" "$$" > "$1"; printf "CAPTURE_SESSION_LEADER=%s\n" "$$" >&2; shift; exec "$@"' capture-session "$pid_file" "$@"
}
