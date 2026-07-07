#!/usr/bin/env bash
# sweep.sh — for each ROUNDS value, run bench.sh multiple times,
#            discard the first run (warm-up: CRS fetch / cold cache),
#            and record the MEDIAN of the rest. This removes the
#            first-touch contamination we observed earlier.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CIRCUIT_SRC="$ROOT_DIR/circuits/balance_threshold/src/main.nr"
SUMMARY="$ROOT_DIR/results/sweep_summary.csv"

REPEATS=3   # runs per condition; first is discarded as warm-up

echo "rounds,constraints,prove_time_ms_median,prove_mem_mb_median" > "$SUMMARY"

# median of space-separated integers passed as args
median() {
    printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END{print (NR%2)? a[(NR+1)/2] : int((a[NR/2]+a[NR/2+1])/2)}'
}

for rounds in 64 65 66 67 68 69 70; do
    echo "########## ROUNDS = $rounds ##########"
    sed -i "s/global ROUNDS: u32 = .*/global ROUNDS: u32 = ${rounds};/" "$CIRCUIT_SRC"

    times=()
    mems=()
    con=""
    for run in $(seq 1 "$REPEATS"); do
        bash "$SCRIPT_DIR/bench.sh" >/dev/null   # silence per-run output
        latest="$(ls -t "$ROOT_DIR/results"/bench_*.json | head -1)"
        # discard first run (warm-up)
        if [ "$run" -gt 1 ]; then
            times+=("$(jq '.prove_time_ms'      "$latest")")
            mems+=("$(jq '.peak_mem_mb.prove | floor' "$latest")")
        fi
        con="$(jq '.constraints' "$latest")"
    done

    pt_med="$(median "${times[@]}")"
    pm_med="$(median "${mems[@]}")"
    echo "  -> constraints=$con  prove_median=${pt_med}ms  mem_median=${pm_med}MB"
    echo "${rounds},${con},${pt_med},${pm_med}" >> "$SUMMARY"
done

echo "========================================"
echo "sweep done (warm-up discarded, median of ${REPEATS} runs):"
column -t -s, "$SUMMARY"