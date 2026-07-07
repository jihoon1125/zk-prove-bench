//! Orchestrates the full measurement run for one circuit.
//!
//! Stages (mirrors bench.sh):
//!   - warm-up compile: NOT measured, keeps compile time out of witness time
//!   - circuit size: `bb gates` -> constraints + acir_opcodes
//!   - repeated loop: measure witness (nargo execute) + prove (bb prove)
//!
//! Reproducibility: repeat `repeats` times, drop the first (cold) run, and
//! reduce the rest to their median.

use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use std::ffi::OsString;
use std::future::Future;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use crate::circuit::Circuit;
use crate::measure::{Sample, measure, median};
use crate::report;
use crate::tools;

/// The proving scheme we pin for M1. (Backend comparison is a later milestone.)
pub const BACKEND: &str = "ultra_honk";

/// Everything the report needs after a run. Time/memory values here are the
/// medians over the kept samples.
pub struct Outcome {
    pub circuit: String,
    pub backend: &'static str,
    pub constraints: u64,
    pub acir_opcodes: u64,
    pub witness: Sample,
    pub prove: Sample,
    pub repeats: usize,
    pub samples_used: usize,
    pub nargo_version: String,
    pub bb_version: String,
}

/// Entry point for `zkbench run`.
pub async fn run(circuit_dir: &Path, repeats: usize) -> Result<()> {
    if repeats == 0 {
        bail!("--repeats must be at least 1");
    }

    // Preflight probes nargo/bb concurrently and captures versions; this is the
    // only concurrency in the tool, and it is safe because it runs before any
    // measurement. Circuit validation is independent, so kick it off too.
    let (versions, circuit) =
        tokio::try_join!(tools::preflight(), Circuit::load(circuit_dir))?;

    // Stage 0: warm-up compile (discarded — only to produce target/<name>.json
    // so the measured `nargo execute` below does not also pay compile cost).
    with_spinner("Compiling (warm-up)...", compile(&circuit)).await?;

    // Stage 1: circuit size. Structural, so measured once.
    let (constraints, acir_opcodes) =
        with_spinner("Reading circuit size...", read_gates(&circuit)).await?;

    // Stage 2: repeated witness + prove measurements.
    tokio::fs::create_dir_all(&circuit.proof_dir)
        .await
        .context("failed to create proof output directory")?;

    let mut witness_samples = Vec::with_capacity(repeats);
    let mut prove_samples = Vec::with_capacity(repeats);
    for i in 1..=repeats {
        // MEASUREMENT IS DELIBERATELY SERIAL. Even though these are async, we
        // await one process fully before starting the next. Running witness and
        // prove (or several repeats) concurrently would make them contend for
        // CPU and memory bandwidth, corrupting both the wall-clock and peak-RSS
        // numbers — which defeats the entire purpose of the tool.
        let witness = with_spinner(
            &format!("Measuring witness ({i}/{repeats})..."),
            // nargo execute resolves the package from the cwd.
            measure("nargo", ["execute"], &circuit.dir),
        )
        .await?;
        witness_samples.push(witness);

        let prove_args: Vec<OsString> = vec![
            "prove".into(),
            "-s".into(),
            BACKEND.into(),
            "-b".into(),
            circuit.bytecode.clone().into_os_string(),
            "-w".into(),
            circuit.witness.clone().into_os_string(),
            "-o".into(),
            circuit.proof_dir.clone().into_os_string(),
        ];
        let prove = with_spinner(
            &format!("Measuring prove ({i}/{repeats})..."),
            measure("bb", prove_args, &circuit.dir),
        )
        .await?;
        prove_samples.push(prove);
    }

    // Drop the first (cold) sample when we have more than one; the first run
    // pays the CRS download / cold-cache cost we explicitly want to exclude.
    let dropped = if repeats > 1 { 1 } else { 0 };
    let witness = reduce(&witness_samples[dropped..]);
    let prove = reduce(&prove_samples[dropped..]);

    let outcome = Outcome {
        circuit: circuit.name.clone(),
        backend: BACKEND,
        constraints,
        acir_opcodes,
        witness,
        prove,
        repeats,
        samples_used: repeats - dropped,
        nargo_version: versions.nargo,
        bb_version: versions.bb,
    };

    report::print(&outcome);
    let saved = report::save_json(&outcome)
        .await
        .context("failed to write result JSON")?;
    report::print_saved_path(&saved);
    Ok(())
}

/// Reduce a set of samples to per-metric medians.
fn reduce(samples: &[Sample]) -> Sample {
    let times: Vec<u64> = samples.iter().map(|s| s.time_ms).collect();
    let mems: Vec<u64> = samples.iter().map(|s| s.peak_kb).collect();
    Sample {
        time_ms: median(&times),
        peak_kb: median(&mems),
    }
}

/// Warm-up compile (not measured). Surfaces nargo's stderr on failure.
async fn compile(circuit: &Circuit) -> Result<()> {
    let out = Command::new("nargo")
        .arg("compile")
        .current_dir(&circuit.dir)
        .output()
        .await
        .context("failed to run nargo compile")?;
    if !out.status.success() {
        bail!("compile failed:\n{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Extract circuit size from `bb gates` JSON:
///   circuit_size = backend (UltraHonk) gate/constraint count
///   acir_opcodes = ACIR-level opcode count
async fn read_gates(circuit: &Circuit) -> Result<(u64, u64)> {
    let out = Command::new("bb")
        .args(["gates", "-s", BACKEND, "-b"])
        .arg(&circuit.bytecode)
        .output()
        .await
        .context("failed to run bb gates")?;
    if !out.status.success() {
        bail!("bb gates failed:\n{}", String::from_utf8_lossy(&out.stderr).trim());
    }

    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("failed to parse bb gates JSON")?;
    let func = &json["functions"][0];
    let constraints = func["circuit_size"]
        .as_u64()
        .context("bb gates output has no circuit_size")?;
    let acir_opcodes = func["acir_opcodes"]
        .as_u64()
        .context("bb gates output has no acir_opcodes")?;
    Ok((constraints, acir_opcodes))
}

/// Run `fut` while showing an indicatif spinner; mark ✓/✗ on completion.
async fn with_spinner<T>(message: &str, fut: impl Future<Output = Result<T>>) -> Result<T> {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}").expect("valid spinner template"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));

    let result = fut.await;
    match &result {
        Ok(_) => pb.finish_with_message(format!("✓ {message}")),
        Err(_) => pb.finish_with_message(format!("✗ {message}")),
    }
    result
}
