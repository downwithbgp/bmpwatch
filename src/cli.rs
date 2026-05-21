use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

use crate::doctor::Doctor;
use crate::event::max_exit_code;
use crate::report;

const DEFAULT_MAX_FINDINGS: usize = 1000;

#[derive(Parser)]
#[command(name = "bmpdoctor", version, about = "BMP file diagnostic tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Human-readable summary of BMP file contents and health
    Inspect {
        /// Path to BMP file
        file: PathBuf,
        /// Cap findings at N (default 1000)
        #[arg(long, default_value_t = DEFAULT_MAX_FINDINGS)]
        max_findings: usize,
        /// Output machine-readable JSON summary instead of text
        #[arg(long)]
        summary_json: bool,
    },
    /// Machine-oriented lint output with exit codes
    Lint {
        /// Path to BMP file
        file: PathBuf,
        /// Cap findings at N (default 1000)
        #[arg(long, default_value_t = DEFAULT_MAX_FINDINGS)]
        max_findings: usize,
    },
    /// Debug/development JSONL output
    Dump {
        /// Path to BMP file
        file: PathBuf,
        /// Emit one JSON object per message
        #[arg(long)]
        jsonl: bool,
        /// Cap findings at N (default 1000)
        #[arg(long, default_value_t = DEFAULT_MAX_FINDINGS)]
        max_findings: usize,
    },
}

pub fn run() {
    let cli = Cli::parse();

    match cli.command {
        Command::Inspect {
            file,
            max_findings,
            summary_json,
        } => {
            let mut doctor = match Doctor::with_max_findings(&file, max_findings.max(1)) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error opening file: {e}");
                    process::exit(1);
                }
            };
            if let Err(e) = doctor.process(false) {
                eprintln!("Error processing file: {e}");
                process::exit(1);
            }
            if summary_json {
                report::render_inspect_json(&doctor.state, doctor.was_truncated());
            } else {
                report::render_inspect(&doctor.state, doctor.was_truncated());
            }
        }
        Command::Lint { file, max_findings } => {
            let mut doctor = match Doctor::with_max_findings(&file, max_findings.max(1)) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error opening file: {e}");
                    process::exit(1);
                }
            };
            if let Err(e) = doctor.process(false) {
                eprintln!("Error processing file: {e}");
                process::exit(1);
            }
            report::render_lint(&doctor.state.findings, doctor.was_truncated());
            process::exit(max_exit_code(&doctor.state.findings));
        }
        Command::Dump {
            file,
            jsonl: true,
            max_findings,
        } => {
            let mut doctor = match Doctor::with_max_findings(&file, max_findings.max(1)) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error opening file: {e}");
                    process::exit(1);
                }
            };
            if let Err(e) = doctor.process(true) {
                eprintln!("Error processing file: {e}");
                process::exit(1);
            }
            doctor.dump_jsonl();
            if doctor.was_truncated() {
                eprintln!(
                    "NOTE: findings truncated at {} (use --max-findings to raise)",
                    max_findings
                );
            }
        }
        Command::Dump { .. } => {
            eprintln!("dump requires --jsonl flag");
            process::exit(1);
        }
    }
}
