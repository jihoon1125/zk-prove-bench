//! Human-facing output (comfy-table + owo-colors) and JSON persistence.

use anyhow::{Context, Result};
use chrono::Local;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table, presets};
use owo_colors::OwoColorize;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::pipeline::Outcome;

/// KB -> MB with two-decimal rounding (for display and the JSON `_mb` fields).
fn kb_to_mb(kb: u64) -> f64 {
    (kb as f64 / 1024.0 * 100.0).round() / 100.0
}

/// Print the full report: header, facts, per-stage table, and one interpretation line.
pub fn print(o: &Outcome) {
    println!();
    println!("{}", format!("● {}", o.circuit).bold());
    println!(
        "  backend {}   nargo {}   bb {}",
        o.backend.dimmed(),
        o.nargo_version.dimmed(),
        o.bb_version.dimmed()
    );
    println!(
        "  constraints {}   acir_opcodes {}   (median of {} run(s), {} repeat(s))",
        o.constraints.to_string().bold(),
        o.acir_opcodes.to_string().bold(),
        o.samples_used,
        o.repeats,
    );
    println!();

    // Per-stage measurement table. The prove row is colored by how heavy it is
    // relative to witness, so the expensive stage stands out at a glance.
    let ratio = heaviness_ratio(o);
    let prove_color = if ratio >= 2.0 {
        Color::Red
    } else if ratio >= 1.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("stage").add_attribute(Attribute::Bold),
            Cell::new("wall-clock (ms)").add_attribute(Attribute::Bold),
            Cell::new("peak mem (MB)").add_attribute(Attribute::Bold),
        ]);

    table.add_row(vec![
        Cell::new("witness"),
        Cell::new(o.witness.time_ms),
        Cell::new(format!("{:.2}", kb_to_mb(o.witness.peak_kb))),
    ]);
    table.add_row(vec![
        Cell::new("prove").fg(prove_color),
        Cell::new(o.prove.time_ms).fg(prove_color),
        Cell::new(format!("{:.2}", kb_to_mb(o.prove.peak_kb))).fg(prove_color),
    ]);

    println!("{table}");

    // One-line interpretation, colored to match the prove row.
    let mem_ratio = o.prove.peak_kb as f64 / o.witness.peak_kb.max(1) as f64;
    let line = format!(
        "→ prove is {ratio:.1}x heavier than witness in time ({mem_ratio:.1}x in memory)"
    );
    let colored = if ratio >= 2.0 {
        line.red().to_string()
    } else if ratio >= 1.0 {
        line.yellow().to_string()
    } else {
        line.green().to_string()
    };
    println!("{colored}");
}

/// Time heaviness of prove relative to witness (guarded against divide-by-zero).
fn heaviness_ratio(o: &Outcome) -> f64 {
    o.prove.time_ms as f64 / o.witness.time_ms.max(1) as f64
}

pub fn print_saved_path(path: &Path) {
    println!("{} {}", "saved:".dimmed(), path.display());
}

// ── JSON persistence ─────────────────────────────────────────────────────
// Shape kept consistent with bench.sh output so any downstream viz can read
// both. Raw KB values are retained alongside the rounded MB values.

#[derive(Serialize)]
struct ToolVersions {
    nargo: String,
    bb: String,
}

#[derive(Serialize)]
struct MemMb {
    witness: f64,
    prove: f64,
}

#[derive(Serialize)]
struct MemKb {
    witness: u64,
    prove: u64,
}

#[derive(Serialize)]
struct BenchResult {
    circuit: String,
    backend: String,
    timestamp: String,
    repeats: usize,
    samples_used: usize,
    tool_versions: ToolVersions,
    constraints: u64,
    acir_opcodes: u64,
    witness_time_ms: u64,
    prove_time_ms: u64,
    peak_mem_mb: MemMb,
    peak_mem_kb: MemKb,
}

/// Write the result to `results/<circuit>_<timestamp>.json` (relative to the
/// current working directory) and return the path.
pub async fn save_json(o: &Outcome) -> Result<PathBuf> {
    let now = Local::now();

    let result = BenchResult {
        circuit: o.circuit.clone(),
        backend: o.backend.to_string(),
        timestamp: now.to_rfc3339(),
        repeats: o.repeats,
        samples_used: o.samples_used,
        tool_versions: ToolVersions {
            nargo: o.nargo_version.clone(),
            bb: o.bb_version.clone(),
        },
        constraints: o.constraints,
        acir_opcodes: o.acir_opcodes,
        witness_time_ms: o.witness.time_ms,
        prove_time_ms: o.prove.time_ms,
        peak_mem_mb: MemMb {
            witness: kb_to_mb(o.witness.peak_kb),
            prove: kb_to_mb(o.prove.peak_kb),
        },
        peak_mem_kb: MemKb {
            witness: o.witness.peak_kb,
            prove: o.prove.peak_kb,
        },
    };

    let dir = PathBuf::from("results");
    tokio::fs::create_dir_all(&dir)
        .await
        .context("failed to create results/ directory")?;
    let filename = format!("{}_{}.json", o.circuit, now.format("%Y%m%dT%H%M%S"));
    let path = dir.join(filename);

    let json = serde_json::to_string_pretty(&result).context("failed to serialize result")?;
    tokio::fs::write(&path, json)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}
