# zkbench

A tool to **profile the proof-generation process** of ZK circuits written in Noir.

It measures, stage by stage, "where and how long proof generation takes
(witness generation vs. proof generation) and how much memory it uses," and
saves those raw numbers as structured JSON. The end goal is to visualize this
data in a dashboard, but **this repository is stage 1 of that — a minimal
viable version that measures a single circuit and stores the result as JSON.**

> Intentionally absent (for now): visualization/dashboard, multi-circuit
> comparison. Those are next steps.

## Requirements

- [Noir](https://noir-lang.org/) (`nargo`) — verified with `1.0.0-beta.3`
- [Barretenberg](https://github.com/AztecProtocol/barretenberg) (`bb`) — verified with `0.82.2`
- `jq`, GNU `/usr/bin/time` (used for peak memory measurement)

Different versions may change `bb`'s subcommand syntax. Check with
`nargo --version` / `bb --version`, and adjust the `bb` calls in
`scripts/bench.sh` if needed.

## Layout

```
zk-prove-bench/
├── circuits/
│   └── balance_threshold/     circuit under test; the unit for adding/swapping circuits
│       ├── Nargo.toml         package definition
│       ├── Prover.toml        proving inputs (balance, threshold) <- parameter swap point
│       └── src/main.nr        balance threshold proof circuit
├── scripts/
│   └── bench.sh               measurement pipeline (the core)
└── results/
    └── bench_<circuit>_<timestamp>.json   where results accumulate
```

## Circuit under test: `balance_threshold`

```rust
fn main(balance: u64, threshold: pub u64) {
    assert(balance >= threshold);
}
```

A minimal payment / stablecoin example — without revealing the actual balance
(`balance`, private), it proves only the fact that "the balance is at or above
a threshold" (`threshold`, public).

## Running

```bash
# Default: measures circuits/balance_threshold
./scripts/bench.sh

# To measure a different circuit, pass its directory
./scripts/bench.sh circuits/<other_circuit>
```

To change the inputs (balance/threshold), edit only
`circuits/balance_threshold/Prover.toml`.

### What the pipeline does

1. **warm-up compile** (`nargo compile`) — not measured. Compiles up front so
   compile time does not leak into the witness measurement.
2. **witness generation** (`nargo execute`) — measures wall-clock time + peak memory
3. **circuit size** (`bb gates`) — extracts ACIR opcode count + gate (constraint) count
4. **proof generation** (`bb prove`) — measures wall-clock time + peak memory
5. writes the result to `results/bench_<circuit>_<timestamp>.json`

## Result JSON field meanings

```json
{
  "circuit": "balance_threshold",     // circuit name (package name)
  "backend": "ultra_honk",            // proving scheme
  "timestamp": "2026-...",            // measurement time
  "tool_versions": { "nargo": "...", "bb": "..." },  // versions for reproducibility
  "constraints": 2810,                // backend (UltraHonk) gate/constraint count = key circuit-size metric
  "acir_opcodes": 6,                  // ACIR-level opcode count (higher-level, before backend expansion)
  "witness_time_ms": 180,             // witness generation wall-clock (ms)
  "prove_time_ms": 110,               // proof generation wall-clock (ms)
  "peak_mem_mb": {                    // per-stage peak physical memory (MB, human-readable)
    "witness": 87.16,
    "prove": 61.81
  },
  "peak_mem_kb": {                    // same values, raw (KB, precision first)
    "witness": 89248,
    "prove": 63292
  }
}
```

### How things are measured (exactly what is captured)

- **Time**: `/usr/bin/time`'s `%e` = the process's **wall-clock elapsed time**
  (seconds). Not CPU time — wall-clock, so it includes I/O and waiting, i.e.
  the perceived latency.
- **Peak memory**: `/usr/bin/time`'s `%M` = **maximum resident set size**
  (`ru_maxrss`, KB). The max physical memory the process held over its
  lifetime — not total heap, but the peak actually resident in RAM.
- Time and memory are captured in the **same single process run**.
- witness and prove are **separate processes**, so their time/memory never mix.

### Notes on interpreting the numbers

- **The first ever `bb prove` run fetches the CRS (common reference string)
  from the internet, which can inflate the time.** For a representative value,
  run once to cache the CRS, then use the second run's value.
- Values jitter run to run (scheduling / cache state). Prefer running several
  times and looking at the distribution.
- `constraints` (gate count) is determined by circuit structure, independent of
  inputs. Time and memory, by contrast, depend on machine state.

## Next steps (out of scope for this repo)

- Accumulate result JSON across multiple circuits/parameters
- A dashboard that reads the accumulated JSON to compare time, memory, and circuit size
