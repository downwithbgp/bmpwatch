use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

use crate::doctor::Doctor;
use crate::event::max_exit_code;
use crate::report;

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
    },
    /// Machine-oriented lint output with exit codes
    Lint {
        /// Path to BMP file
        file: PathBuf,
    },
    /// Debug/development JSONL output
    Dump {
        /// Path to BMP file
        file: PathBuf,
        /// Emit one JSON object per message
        #[arg(long)]
        jsonl: bool,
    },
}

pub fn run() {
    let cli = Cli::parse();

    match cli.command {
        Command::Inspect { file } => {
            let mut doctor = match Doctor::new(&file) {
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
            report::render_inspect(&doctor.state);
        }
        Command::Lint { file } => {
            let mut doctor = match Doctor::new(&file) {
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
            report::render_lint(&doctor.state.findings);
            process::exit(max_exit_code(&doctor.state.findings));
        }
        Command::Dump { file, jsonl: true } => {
            let mut doctor = match Doctor::new(&file) {
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
        }
        Command::Dump { .. } => {
            eprintln!("dump requires --jsonl flag");
            process::exit(1);
        }
    }
}
