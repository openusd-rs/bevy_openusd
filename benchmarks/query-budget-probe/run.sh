#!/run/current-system/sw/bin/bash
set -euo pipefail
out=${1:?provide evidence directory}
expected=${2:-49bd693b6530a910be069ac670b98b5cee62c30b}
protocol=${3:-admit}
case "$protocol" in
  admit) policy=admit ;;
  mixed) policy=small-sources ;;
  *) echo 'protocol must be admit or mixed' >&2; exit 2 ;;
esac
upstream=/tmp/bevy-optimization-query-budget.uW9rMA/upstream
trace=/tmp/bevy-optimization-animation-access.tzNL5k
node=/nix/store/v2210ar51jddnrcx46m53ivv9palc26m-nodejs-24.14.0/bin/node
timer=/run/current-system/sw/bin/time
test "$(git -C "$upstream" rev-parse HEAD)" = "$expected"
git -C "$upstream" diff --exit-code
git -C "$upstream" rev-parse HEAD > "$out/upstream-head"
git -C "$upstream" diff 900ea31591348e3fbf416d4661d703aa8700771b HEAD --binary > "$out/upstream.patch"
for variable in "${!USD_@}"; do unset "$variable"; done
cp target/release/query_budget_probe "$out/probe"
cp "$trace/analysis.json" "$out/trace-analysis.json"
cp "$trace/assets.sha256" "$out/assets.sha256"
sha256sum "$out"/{probe,trace-analysis.json,upstream-head,upstream.patch} Cargo.lock Cargo.toml crates/usd_bevy/Cargo.toml \
  crates/usd_bevy/src/source.rs .make.lua benchmarks/patches/openusd-query-budget.patch \
  benchmarks/query-budget-probe/{Cargo.toml,Cargo.lock,src/main.rs,analyze.mjs,negative.mjs,run.sh,bird.paths,cow.paths} > "$out/inputs.sha256"
sha256sum -c "$out/assets.sha256" > "$out/assets-before.log"
sha256sum -c "$out/inputs.sha256" > "$out/inputs-before.log"
{ date -Ins; uname -sr; grep -m1 'model name' /proc/cpuinfo; grep '^Cpus_allowed_list:' /proc/$$/status; } > "$out/environment.txt"
printf 'case\tstatus\n' > "$out/results.tsv"
while read -r name asset reverse; do
  kind=${name%-*}
  { date -Ins; cat /proc/loadavg /proc/pressure/cpu /proc/pressure/memory; ps -eo pid,ppid,comm,args | grep -E 'cargo (build|test)|rustc|usdview|editor_benchmark' | grep -v grep || true; } > "$out/$name.host-before.txt"
  printf '%s %s begin\n' "$(date -Ins)" "$name" | tee -a "$out/progress.log"
  status=0
  oslo make run-headless-benchmark --binary "$timer" \
    --args="-f 'elapsed_s=%e user_s=%U sys_s=%S peak_rss_kb=%M exit=%x' -o $out/$name.time $out/probe $asset benchmarks/query-budget-probe/$kind.paths $reverse $policy >$out/$name.jsonl 2>$out/$name.stderr" \
    > "$out/$name.launcher.log" 2>&1 || status=$?
  printf '%s\n' "$status" > "$out/$name.exit"
  printf '%s\t%s\n' "$name" "$status" >> "$out/results.tsv"
  { date -Ins; cat /proc/loadavg /proc/pressure/cpu /proc/pressure/memory; } > "$out/$name.host-after.txt"
  if [[ "$status" != 0 ]]; then exit "$status"; fi
  if [[ -s "$out/$name.stderr" ]]; then cat "$out/$name.stderr"; exit 1; fi
  printf '%s %s complete\n' "$(date -Ins)" "$name" | tee -a "$out/progress.log"
done <<'CASES'
bird-0 assets/external/hummingbird.usdz 0
cow-0 assets/external/cows/cow_eating.usda 0
cow-1 assets/external/cows/cow_eating.usda 1
bird-1 assets/external/hummingbird.usdz 1
CASES
sha256sum -c "$out/assets.sha256" > "$out/assets-after.log"
sha256sum -c "$out/inputs.sha256" > "$out/inputs-after.log"
git -C "$upstream" diff --exit-code
status=0
oslo make run-headless-benchmark --binary "$node" \
  --args="benchmarks/query-budget-probe/analyze.mjs $out $out/trace-analysis.json $protocol" > "$out/analysis.log" 2>&1 || status=$?
printf '%s\n' "$status" > "$out/analysis.exit"
oslo make run-headless-benchmark --binary "$node" \
  --args="benchmarks/query-budget-probe/negative.mjs $out" > "$out/negative.log" 2>&1
exit "$status"
