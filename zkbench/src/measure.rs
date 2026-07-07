//! Subprocess measurement: wall-clock time + peak RSS, plus a median helper.
//!
//! This ports the methodology from the repo's `bench.sh`: wrap the target
//! command in GNU `/usr/bin/time -f '%e %M'`, where
//!   %e = elapsed wall-clock time (seconds)
//!   %M = maximum resident set size = peak RSS over the process lifetime (KB)
//! We measure from outside because nargo/bb are external binaries — the OS is
//! the only place that sees their true peak memory.
//!
//! `measure` is async (it awaits the child via tokio), but callers must still
//! invoke it one process at a time: running measured processes concurrently
//! would make them contend for CPU and memory and corrupt the numbers.

use anyhow::{Context, Result, bail};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// One raw measurement of a single subprocess run.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub time_ms: u64,
    pub peak_kb: u64,
}

/// Run `program args...` under `/usr/bin/time` with `cwd` as the working
/// directory, returning its wall-clock time and peak RSS.
///
/// `/usr/bin/time`'s stats are written to a dedicated temp file via `-o`, so
/// they never mix with the child's stdout/stderr and parsing stays clean.
/// The child's stdout is discarded; its stderr is captured so that, on
/// failure, we can surface the real error message.
pub async fn measure<I, S>(program: &str, args: I, cwd: &Path) -> Result<Sample>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let stats = tempfile::NamedTempFile::new()
        .context("failed to create temp file for time stats")?;

    let output = Command::new("/usr/bin/time")
        .arg("-f")
        .arg("%e %M")
        .arg("-o")
        .arg(stats.path())
        .arg("--")
        .arg(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn /usr/bin/time")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`{program}` failed ({}):\n{}", output.status, stderr.trim());
    }

    let raw = tokio::fs::read_to_string(stats.path())
        .await
        .context("failed to read /usr/bin/time stats file")?;
    parse_time_output(&raw)
        .with_context(|| format!("could not parse /usr/bin/time output: {raw:?}"))
}

/// Parse the "`%e %M`" line, e.g. "0.18 89248" -> Sample.
///
/// We read the last two whitespace-separated tokens defensively; the stats
/// file normally holds exactly "<seconds> <kilobytes>".
fn parse_time_output(raw: &str) -> Option<Sample> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let wall_s: f64 = tokens[tokens.len() - 2].parse().ok()?;
    let peak_kb: u64 = tokens[tokens.len() - 1].parse().ok()?;
    Some(Sample {
        // Source precision is 2-decimal seconds, so ms is the meaningful unit.
        time_ms: (wall_s * 1000.0).round() as u64,
        peak_kb,
    })
}

/// Median of a slice, used to collapse repeated samples into one number.
/// For an even count, returns the average of the two middle values.
pub fn median(values: &[u64]) -> u64 {
    assert!(!values.is_empty(), "median of empty slice");
    let mut v = values.to_vec();
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd_returns_middle() {
        assert_eq!(median(&[3, 1, 2]), 2);
    }

    #[test]
    fn median_even_averages_two_middle() {
        assert_eq!(median(&[10, 20, 30, 40]), 25);
    }

    #[test]
    fn median_single_element() {
        assert_eq!(median(&[42]), 42);
    }

    #[test]
    fn parse_time_typical_line() {
        let s = parse_time_output("0.18 89248").unwrap();
        assert_eq!(s.time_ms, 180);
        assert_eq!(s.peak_kb, 89248);
    }

    #[test]
    fn parse_time_rounds_seconds_to_ms() {
        // 1.235 s -> 1235 ms
        let s = parse_time_output("1.235 1000").unwrap();
        assert_eq!(s.time_ms, 1235);
    }

    #[test]
    fn parse_time_tolerates_trailing_newline_and_padding() {
        let s = parse_time_output("  0.50 2048\n").unwrap();
        assert_eq!(s.time_ms, 500);
        assert_eq!(s.peak_kb, 2048);
    }

    #[test]
    fn parse_time_rejects_garbage() {
        assert!(parse_time_output("").is_none());
        assert!(parse_time_output("oops").is_none());
        assert!(parse_time_output("abc def").is_none());
    }
}
