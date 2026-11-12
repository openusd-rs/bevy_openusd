#!/run/current-system/sw/bin/bash
set -euo pipefail
printf 'owned_app_pid=%s\n' "$$"
exec "$@"
