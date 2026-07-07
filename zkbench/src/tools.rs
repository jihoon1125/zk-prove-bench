//! External-tool preflight (nargo, bb, /usr/bin/time) and version capture.
//!
//! We validate the toolchain up front so the user gets a friendly message
//! instead of a cryptic "exit 127" surfacing mid-measurement.
//!
//! This is the one place concurrency is a genuine win: the nargo/bb `--version`
//! probes are independent and run *before* any measurement, so running them
//! together via `tokio::try_join!` cannot perturb the numbers we care about.

use anyhow::{Context, Result, bail};
use std::io::ErrorKind;
use std::path::Path;
use tokio::process::Command;

/// Versions of the underlying tools, recorded for reproducibility.
pub struct Versions {
    pub nargo: String,
    pub bb: String,
}

/// Validate the toolchain and capture versions. `nargo` and `bb` are probed
/// concurrently; `/usr/bin/time` is a cheap local path check.
pub async fn preflight() -> Result<Versions> {
    ensure_time()?;

    // Independent probes -> run them at the same time. try_join! short-circuits
    // on the first error (e.g. a missing tool) and returns its friendly message.
    let (nargo_raw, bb_raw) = tokio::try_join!(tool_version("nargo"), tool_version("bb"))?;

    // nargo prints e.g. "nargo version = 1.0.0-beta.3" on its first line.
    let nargo = nargo_raw
        .lines()
        .next()
        .and_then(|line| line.split('=').nth(1))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| nargo_raw.trim().to_string());

    // bb prints just the bare version, e.g. "0.82.2".
    let bb = bb_raw.trim().to_string();

    Ok(Versions { nargo, bb })
}

/// Run `<name> --version`, mapping the "not installed" case to a user-facing
/// message. Doubles as both the existence check and the version capture.
async fn tool_version(name: &str) -> Result<String> {
    match Command::new(name).arg("--version").output().await {
        Ok(out) => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            bail!("cannot find `{name}` — please check it is installed")
        }
        Err(e) => Err(e).with_context(|| format!("failed to run {name}")),
    }
}

/// GNU `/usr/bin/time` is required for the `%e %M` measurement format used by
/// [`crate::measure`]. The shell built-in `time` does not support it.
fn ensure_time() -> Result<()> {
    if Path::new("/usr/bin/time").exists() {
        Ok(())
    } else {
        bail!("cannot find /usr/bin/time — please check it is installed (GNU time required)")
    }
}
