use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

use crate::dashboard;
use crate::doctor::Doctor;
use crate::event::max_exit_code;
use crate::input::{detect_format, InputFormat};
use crate::report;

use crate::doctor;

const DEFAULT_MAX_FINDINGS: usize = 1000;

fn resolve_format(file: &Path, format: InputFormat) -> InputFormat {
    match format {
        InputFormat::Auto => match detect_format(file) {
            Ok(fmt) => fmt,
            Err(e) => {
                eprintln!("Error detecting format: {e}, falling back to raw-bmp");
                InputFormat::RawBmp
            }
        },
        explicit => explicit,
    }
}

#[derive(Parser)]
#[command(
    name = "bmpwatch",
    version,
    about = "BMP stream monitoring and diagnostic tool",
    subcommand_negates_reqs = true,
)]
pub struct Cli {
    /// Path to a .bmpd or .rawbmp capture file (replay mode).
    /// If omitted, opens the live Kafka dashboard.
    #[arg()]
    pub file: Option<PathBuf>,

    /// Rolling window size for replay mode (default 10)
    #[arg(long, default_value_t = 10)]
    window_messages: usize,

    /// Summary emission interval in milliseconds for replay mode (default 1000)
    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,

    /// Input format for replay mode: auto, raw-bmp, or bmpd
    #[arg(long, default_value = "auto")]
    format: InputFormat,

    /// Kafka broker address (dashboard mode)
    #[arg(long, default_value = "stream.routeviews.org:9092")]
    broker: String,

    /// Exact Kafka topic name — skip the stream browser
    #[arg(long)]
    topic: Option<String>,

    /// Pre-filter topics by collector/router fragment
    #[arg(long)]
    collector: Option<String>,

    /// Pre-filter topics by ASN
    #[arg(long)]
    asn: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
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
        /// Input format: auto, raw-bmp, or bmpd
        #[arg(long, default_value = "auto")]
        format: InputFormat,
        /// Maximum peers to display (0 suppresses peer sections)
        #[arg(long, default_value_t = 10)]
        max_peers: usize,
    },
    /// Machine-oriented lint output with exit codes
    Lint {
        /// Path to BMP file
        file: PathBuf,
        /// Cap findings at N (default 1000)
        #[arg(long, default_value_t = DEFAULT_MAX_FINDINGS)]
        max_findings: usize,
        /// Input format: auto, raw-bmp, or bmpd
        #[arg(long, default_value = "auto")]
        format: InputFormat,
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
        /// Input format: auto, raw-bmp, or bmpd
        #[arg(long, default_value = "auto")]
        format: InputFormat,
    },
    /// Live TUI dashboard for a RouteViews BMP stream (the default)
    Dashboard {
        /// Kafka broker address
        #[arg(long, default_value = "stream.routeviews.org:9092")]
        broker: String,
        /// Exact Kafka topic name (skip topic browser)
        #[arg(long)]
        topic: Option<String>,
        /// Pre-filter topics by collector/router fragment
        #[arg(long)]
        collector: Option<String>,
        /// Pre-filter topics by ASN
        #[arg(long)]
        asn: Option<String>,
        /// Rolling window size in messages
        #[arg(long, default_value_t = 100)]
        window_messages: usize,
    },
}

pub fn run() {
    let cli = Cli::parse();

    // No subcommand: file → replay mode, no file → dashboard
    if cli.command.is_none() {
        if let Some(ref file) = cli.file {
            let fmt = resolve_format(file, cli.format);
            if let Err(e) = doctor::watch(file, cli.window_messages, cli.interval_ms, fmt) {
                eprintln!("Error in replay mode: {e}");
                process::exit(1);
            }
            return;
        } else {
            // Default: live dashboard
            if let Err(e) = dashboard::run_dashboard(
                &cli.broker,
                cli.topic.as_deref(),
                cli.collector.as_deref(),
                cli.asn.as_deref(),
                100,
            ) {
                eprintln!("Dashboard error: {e}");
                process::exit(1);
            }
            return;
        }
    }

    match cli.command.unwrap() {
        Command::Inspect {
            file,
            max_findings,
            summary_json,
            format,
            max_peers,
        } => {
            let fmt = resolve_format(&file, format);
            let mut doctor = match Doctor::with_max_findings(&file, max_findings.max(1), fmt) {
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
                report::render_inspect_json(&doctor.state, doctor.was_truncated(), max_peers);
            } else {
                report::render_inspect(&doctor.state, doctor.was_truncated(), max_peers);
            }
        }
        Command::Lint {
            file,
            max_findings,
            format,
        } => {
            let fmt = resolve_format(&file, format);
            let mut doctor = match Doctor::with_max_findings(&file, max_findings.max(1), fmt) {
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
            report::render_lint(
                &doctor.state.findings,
                doctor.was_truncated(),
                doctor.state.findings_dropped,
            );
            process::exit(max_exit_code(&doctor.state.findings));
        }
        Command::Dashboard {
            broker,
            topic,
            collector,
            asn,
            window_messages,
        } => {
            if let Err(e) = dashboard::run_dashboard(
                &broker,
                topic.as_deref(),
                collector.as_deref(),
                asn.as_deref(),
                window_messages,
            ) {
                eprintln!("Dashboard error: {e}");
                process::exit(1);
            }
        }
        Command::Dump {
            file,
            jsonl: true,
            max_findings,
            format,
        } => {
            let fmt = resolve_format(&file, format);
            let mut doctor = match Doctor::with_max_findings(&file, max_findings.max(1), fmt) {
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
                    "NOTE: findings truncated at {} ({} dropped). Use --max-findings to raise.",
                    max_findings, doctor.state.findings_dropped,
                );
            }
        }
        Command::Dump { .. } => {
            eprintln!("dump requires --jsonl flag");
            process::exit(1);
        }
    }
}
