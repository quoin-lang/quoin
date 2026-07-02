#!/usr/bin/env bash
# Batch-size sweep harness. For each batch size, run a workload with per-batch stats and
# report: per_instr (ns, BENEFIT — falls then flattens; knee = sweet spot), time/batch (us)
# and alloc/batch (GC bytes) (COSTS — grow with batch), and peak RSS (MB, real memory cost).
# Usage: sweep.sh <workload.qn>
set -u
QN=target/release/qn
WL="$1"
SIZES="1 16 64 256 512 1024 4096 16384 65536 262144"
printf "%-9s %11s %13s %14s %10s\n" "batch" "per_instr" "time/batch" "alloc/batch" "peakRSS"
printf "%-9s %11s %13s %14s %10s\n" "(N)" "(ns)" "(us)" "(bytes)" "(MB)"
for N in $SIZES; do
  # best-of-2 by min per_instr; capture stderr (batch-stats + /usr/bin/time -l)
  best_pi=""; line=""; rssmb=""
  for r in 1 2; do
    err=$(QN_BATCH_STATS=1 QN_BATCH="$N" /usr/bin/time -l "$QN" "$WL" 2>&1 >/dev/null)
    bs=$(echo "$err" | grep 'batch-stats' | tail -1)
    pi=$(echo "$bs" | sed -E 's/.*per_instr=[[:space:]]*([0-9.]+)ns.*/\1/')
    rss=$(echo "$err" | awk '/maximum resident set size/{print $1; exit}')
    if [ -z "$best_pi" ] || awk -v a="$pi" -v b="$best_pi" 'BEGIN{exit !(a<b)}'; then
      best_pi="$pi"; line="$bs"; rssmb=$(awk -v b="$rss" 'BEGIN{printf "%.1f", b/1048576}')
    fi
  done
  tb=$(echo "$line"   | sed -E 's/.*time\/batch=[[:space:]]*([0-9.]+)us.*/\1/')
  ab=$(echo "$line"   | sed -E 's/.*alloc\/batch=[[:space:]]*([0-9]+)B.*/\1/')
  printf "%-9s %11s %13s %14s %10s\n" "$N" "$best_pi" "$tb" "$ab" "$rssmb"
done
