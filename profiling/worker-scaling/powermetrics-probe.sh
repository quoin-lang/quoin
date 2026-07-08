#!/bin/bash
# Core-residency probe for the >4-worker in-process scaling ceiling
# (profiling/worker-scaling/notes.md). Run from anywhere:
#
#     sudo profiling/worker-scaling/powermetrics-probe.sh
#
# Three legs, each sampled by powermetrics at 200 ms while the workload
# repeats 3x: (a) 8 in-process workers, (b) 4 in-process workers,
# (c) 8 separate qn processes. The workloads run as the invoking user
# (sudo -u), so scheduling is realistic and no root-owned files land in
# target/.
#
# READING THE RESULT — the P-cluster hypothesis predicts:
#   workers8: ONE P-cluster (and/or E-cluster) saturated, other P-cores
#             largely idle — same footprint as workers4, just oversubscribed.
#   procs8:   ALL P-clusters active.
# If workers8 instead spreads across all P-clusters like procs8 does, the
# hypothesis is WRONG and the ceiling needs a new suspect.
set -euo pipefail
cd "$(dirname "$0")/../.."
QN=./target/release/qn
[ -x "$QN" ] || { echo "build first: cargo build --release"; exit 1; }
[ "$(id -u)" = "0" ] || { echo "run with sudo (powermetrics needs it)"; exit 1; }
RUNAS=${SUDO_USER:-root}
OUT=profiling/worker-scaling

run_leg() {
    local name=$1; shift
    echo "=== leg: $name"
    powermetrics --samplers cpu_power -i 200 > "$OUT/pm-$name.log" 2>/dev/null &
    local PM=$!
    sleep 0.6
    local start end
    start=$(python3 -c 'import time; print(time.time())')
    for i in 1 2 3; do sudo -u "$RUNAS" "$@" > /dev/null 2>&1 || true; done
    end=$(python3 -c 'import time; print(time.time())')
    kill "$PM" 2>/dev/null || true
    wait "$PM" 2>/dev/null || true
    echo "    3 runs took $(python3 -c "print(int(($end - $start) * 1000))") ms"
}

run_leg workers8 "$QN" profiling/worker-scaling/unit_scale.qn 8
run_leg workers4 "$QN" profiling/worker-scaling/unit_scale.qn 4
run_leg procs8 bash -c 'for i in $(seq 1 8); do ./target/release/qn profiling/worker-scaling/control.qn 2500 & done; wait'

echo
echo "================ mean active residency per leg ================"
for name in workers8 workers4 procs8; do
    echo "--- $name"
    awk '/Cluster HW active residency:/ { v = $5; gsub("%", "", v); sum[$1] += v; n[$1]++ }
         END { for (k in sum) printf "  %-12s %5.1f%%\n", k, sum[k] / n[k] }' \
        "$OUT/pm-$name.log" | sort
    awk '/^CPU [0-9]+ active residency:/ { v = $5; gsub("%", "", v); sum[$2] += v; n[$2]++ }
         END { for (k in sum) printf "  CPU %-3s %5.1f%%\n", k, sum[k] / n[k] }' \
        "$OUT/pm-$name.log" | sort -t' ' -k2 -n
done
echo
echo "Logs kept in $OUT/pm-*.log — attach or paste the summary above."
