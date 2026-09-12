#!/bin/bash
set -euo pipefail

if [[ $# != 1 || ! -f "$1" || ! -r "$1" ]]; then
    echo "usage: check_viewer_log.sh READABLE_VIEWER_LOG" >&2
    exit 2
fi
status=0
grep -E "^thread .* panicked at|^Encountered a panic in system|^thread caused non-unwinding panic|^fatal runtime error:|ERROR .*bevy_render::error_handler: Caught rendering error:" -- "$1" >/dev/null || status=$?
case "$status" in
    0) echo "viewer log contains a runtime panic or rendering failure; artifacts retained: $1" >&2; exit 1 ;;
    1) echo "VIEWER_LOG_OK $1 (known panic and rendering-error markers only)" ;;
    *) echo "viewer log inspection failed: $1" >&2; exit 2 ;;
esac
