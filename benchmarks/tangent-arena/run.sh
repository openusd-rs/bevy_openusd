set -euo pipefail
root=$(git rev-parse --show-toplevel)
out=${1:?provide the absolute evidence directory}
[[ "$out" == /* && -d "$out" ]]
cd "$root"
for variable in "${!USD_@}"; do unset "$variable"; done
export USD_TANGENT_SAMPLES=100
sha256sum -c "$out/production-inputs.sha256" > "$out/production-before.log"
sha256sum -c "$out/library-source.sha256" > "$out/library-before.log"
sha256sum -c "$out/binaries.sha256" > "$out/binaries-before.log"
sha256sum -c "$out/candidate.sha256" > "$out/candidate-before-run.log"
sha256sum -c "$out/assets.sha256" > "$out/assets-before.log"
{ date -Ins; uname -sr; grep -m1 '^model name' /proc/cpuinfo; nproc; taskset -pc $$; } > "$out/environment.txt"
printf 'case\trun\tvariant\tstatus\tdiagnostics\n' > "$out/results.tsv"
failed=0
while read -r name asset; do
  export USD_TANGENT_ASSET="$root/$asset"
  run=0
  for variant in before after after before; do
    run=$((run + 1)); id="$name-$run-$variant"
    printf '%s %s\n' "$(date -Ins)" "$id" | tee -a "$out/progress.log"
    cat /proc/loadavg /proc/pressure/cpu /proc/pressure/memory > "$out/$id.host-before.txt"
    ps -eo pid,etimes,pcpu,args > "$out/$id.processes-before.txt"
    status=0
    oslo make run-headless-benchmark --binary /run/current-system/sw/bin/time --args="-f 'wall_s=%e user_s=%U system_s=%S peak_rss_kib=%M exit=%x' -o '$out/$id.time' '$out/$variant-probe' route::preparation::paired_probe::real_asset_tangent_kernel --exact --ignored --nocapture --test-threads=1" > "$out/$id.log" 2>&1 || status=$?
    cat /proc/loadavg /proc/pressure/cpu /proc/pressure/memory > "$out/$id.host-after.txt"
    diagnostics=$(grep -Ec '(^|[[:space:]])(ERROR|WARN)([[:space:]]|$)|panicked at|Error:' "$out/$id.log" || true)
    printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$run" "$variant" "$status" "$diagnostics" >> "$out/results.tsv"
    if [[ "$status" != 0 || "$diagnostics" != 0 ]]; then failed=1; fi
  done
done <<'CASES'
cow assets/external/cows/cow_eating.usda
bird assets/external/hummingbird.usdz
CASES
sha256sum -c "$out/production-inputs.sha256" > "$out/production-after.log"
sha256sum -c "$out/library-source.sha256" > "$out/library-after.log"
sha256sum -c "$out/binaries.sha256" > "$out/binaries-after.log"
sha256sum -c "$out/candidate.sha256" > "$out/candidate-after-run.log"
sha256sum -c "$out/assets.sha256" > "$out/assets-after.log"
printf 'TANGENT_ARENA_COMPLETE failures=%s\n' "$failed" | tee -a "$out/progress.log"
exit "$failed"
