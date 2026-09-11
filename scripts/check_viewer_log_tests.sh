#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
directory=$(mktemp -d)
trap 'rm -rf -- "$directory"' EXIT
log="$directory/viewer log.txt"
check() {
    local expected=$1 actual=0
    shift
    /bin/bash "$root/scripts/check_viewer_log.sh" "$@" > "$directory/result" 2>&1 || actual=$?
    [[ "$actual" == "$expected" ]] || { cat "$directory/result" >&2; echo "expected $expected, got $actual" >&2; exit 1; }
}
printf 'USD_VIEWER_UI_UPDATED\nINFO normal operation\n' > "$log"
check 0 "$log"
for message in "thread 'main' panicked at source.rs:3:4:" "thread '<unnamed>' (123) panicked at source.rs:3:4:" \
    'Encountered a panic in system `example`!' 'thread caused non-unwinding panic. aborting.' 'fatal runtime error: stack overflow'; do
    printf '%s\nUSD_VIEWER_UI_UPDATED\n' "$message" > "$log"
    check 1 "$log"
    ! grep -q 'VIEWER_LOG_OK' "$directory/result"
done
printf 'INFO no panic detected\nINFO discussing panicked at as text\n' > "$log"
check 0 "$log"
: > "$log"
check 0 "$log"
check 2
check 2 "$directory/missing"
check 2 "$directory"
check 2 "$log" extra
echo VIEWER_LOG_TESTS_OK
