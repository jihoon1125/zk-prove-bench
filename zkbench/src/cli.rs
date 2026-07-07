//! Command-line surface for zkbench (clap derive).

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// zkbench — a performance profiler for Noir ZK circuits (nargo + Barretenberg).
#[derive(Parser)]
#[command(name = "zkbench", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Measure proof generation for a single circuit and print a report.
    Run {
        /// Path to the circuit directory (the folder that contains Nargo.toml).
        circuit_dir: PathBuf,

        /// How many times to repeat each measurement. The first run is
        /// discarded (cold CRS download / cold cache), and the rest are
        /// reduced to their median. Minimum 1.
        #[arg(long, default_value_t = 3)]
        repeats: usize,
    },
}
