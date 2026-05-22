use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::Message;

use bmpdoctor::obmp_writer::ObmpWriter;

const DEFAULT_TOPIC_LIMIT: usize = 20;

#[derive(Parser)]
#[command(
    name = "record_openbmp_kafka",
    version,
    about = "Record RouteViews Kafka BMP messages to a local .bmpd file"
)]
struct Cli {
    #[arg(long, default_value = "stream.routeviews.org:9092")]
    broker: String,

    #[arg(long, help = "Exact Kafka topic name (overrides --topic-regex)")]
    topic: Option<String>,

    #[arg(
        long,
        default_value = "^routeviews.*\\.bmp_raw$",
        help = "Regex to match topic names (ignored if --topic is set)"
    )]
    topic_regex: String,

    #[arg(
        long,
        default_value = "samples/routeviews-sample.bmpd",
        help = "Output .bmpd file path"
    )]
    out: PathBuf,

    #[arg(long, default_value = "10", help = "Maximum messages to consume")]
    max_messages: u64,

    #[arg(long, default_value = "30", help = "Maximum seconds to run")]
    max_seconds: u64,

    #[arg(
        long,
        default_value = "true",
        help = "Start consuming from end of topic (latest offsets)"
    )]
    from_end: bool,

    #[arg(long, help = "List matching topics and exit (no recording)")]
    list_topics: bool,

    #[arg(
        long,
        help = "List matching topics as JSON array and exit (no recording)"
    )]
    list_topics_json: bool,

    #[arg(
        long,
        default_value_t = DEFAULT_TOPIC_LIMIT,
        help = "Refuse to subscribe if regex matches more than N topics"
    )]
    topic_limit: usize,

    #[arg(
        long,
        help = "Filter topics containing this collector/router group fragment (e.g. chicago, linx)"
    )]
    collector: Option<String>,

    #[arg(long, help = "Filter topics ending in .<ASN>.bmp_raw (e.g. 13335)")]
    asn: Option<String>,

    #[arg(
        long,
        default_value = "0",
        help = "Exit non-zero if fewer than N messages are captured"
    )]
    min_messages: u64,
}

fn fetch_topics(broker: &str, pattern: &str) -> Result<Vec<String>> {
    let consumer: BaseConsumer = ClientConfig::new()
        .set("bootstrap.servers", broker)
        .set("group.id", "bmpdoctor-recorder-metadata")
        .create()
        .context("Failed to create metadata consumer")?;

    let metadata = consumer
        .fetch_metadata(None, Duration::from_secs(10))
        .context("Failed to fetch topic metadata")?;

    let regex =
        regex::Regex::new(pattern).with_context(|| format!("Invalid topic regex: {pattern}"))?;

    let mut matched: Vec<String> = metadata
        .topics()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|name| regex.is_match(name))
        .collect();
    matched.sort();
    Ok(matched)
}

fn apply_filters(topics: Vec<String>, collector: Option<&str>, asn: Option<&str>) -> Vec<String> {
    let mut v = topics;
    if let Some(frag) = collector {
        let lower = frag.to_lowercase();
        v.retain(|t| t.to_lowercase().contains(&lower));
    }
    if let Some(asn_val) = asn {
        let suffix = format!(".{asn_val}.bmp_raw");
        v.retain(|t| t.ends_with(&suffix));
    }
    v
}

/// Check whether the minimum message threshold is met.
/// Returns Ok(()) if it is, or a descriptive error string if not.
fn check_min_messages(got: u64, required: u64) -> Result<(), String> {
    if required > 0 && got < required {
        Err(format!(
            "Capture failed minimum message requirement: got {got}, required {required}."
        ))
    } else {
        Ok(())
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(ref exact) = cli.topic {
        if cli.list_topics_json {
            println!("[\"{exact}\"]");
            return Ok(());
        }
        if cli.list_topics {
            println!("{exact}");
            return Ok(());
        }
        run_record(&cli, vec![exact.clone()])
    } else {
        let matched = fetch_topics(&cli.broker, &cli.topic_regex)?;

        let filtered = apply_filters(matched, cli.collector.as_deref(), cli.asn.as_deref());

        if filtered.is_empty() {
            let hint = if cli.collector.is_some() || cli.asn.is_some() {
                ". Try removing --collector/--asn filters or broadening --topic-regex."
            } else {
                ". Check --broker and --topic-regex."
            };
            anyhow::bail!("No topics matched after applying filters{hint}");
        }

        if cli.list_topics_json {
            let json = serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".to_string());
            println!("{json}");
            return Ok(());
        }

        if cli.list_topics {
            for t in &filtered {
                println!("{t}");
            }
            eprintln!("{} topics matched", filtered.len());
            return Ok(());
        }

        if filtered.len() > cli.topic_limit {
            let show = filtered.len().min(3);
            let sample: Vec<_> = filtered.iter().take(show).collect();
            anyhow::bail!(
                "Regex matched {} topics (after filters) but --topic-limit is {}.\n\
                 First {} topic(s):\n  {}\n\n\
                 Choose one of:\n  \
                 --topic <exact-topic>       pick a single topic\n  \
                 --collector <fragment>      filter by collector/router name\n  \
                 --asn <ASN>                 filter by peer ASN\n  \
                 --topic-limit <N>           raise the limit\n  \
                 --list-topics              see all matching topics",
                filtered.len(),
                cli.topic_limit,
                show,
                sample
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  "),
            );
        }

        eprintln!(
            "Regex '{}' matched {} topics (limit {})",
            cli.topic_regex,
            filtered.len(),
            cli.topic_limit,
        );
        run_record(&cli, filtered)
    }
}

fn run_record(cli: &Cli, topics: Vec<String>) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl-C handler")?;

    let offset_reset = if cli.from_end { "latest" } else { "earliest" };

    let consumer: BaseConsumer = ClientConfig::new()
        .set("bootstrap.servers", &cli.broker)
        .set("group.id", "bmpdoctor-recorder")
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", offset_reset)
        .set("session.timeout.ms", "10000")
        .create()
        .context("Failed to create Kafka consumer")?;

    let topic_strs: Vec<&str> = topics.iter().map(|s| s.as_str()).collect();
    consumer
        .subscribe(&topic_strs)
        .context("Failed to subscribe to topics")?;

    let mut writer = ObmpWriter::create(&cli.out).context("Failed to create output .bmpd file")?;

    eprintln!(
        "Recording to {} (max {} msgs, {} secs, from_end={})",
        cli.out.display(),
        cli.max_messages,
        cli.max_seconds,
        cli.from_end,
    );

    let start = Instant::now();

    while running.load(Ordering::SeqCst)
        && writer.messages_written() < cli.max_messages
        && start.elapsed().as_secs() < cli.max_seconds
    {
        match consumer.poll(Duration::from_millis(500)) {
            Some(Ok(msg)) => {
                if let Some(payload) = msg.payload() {
                    writer
                        .write_frame(payload)
                        .context("Failed to write frame")?;
                }
            }
            Some(Err(e)) => {
                eprintln!("Kafka error: {e}");
            }
            None => {}
        }
    }

    let msg_count = writer.messages_written();
    let byte_count = writer.bytes_written();
    writer.finish().context("Failed to finalize output file")?;

    println!("broker: {}", cli.broker);
    println!("topic/s: {}", topics.join(", "));
    println!("messages_written: {msg_count}");
    println!("bytes_written: {byte_count}");
    println!("output_path: {}", cli.out.display());
    println!("duration_secs: {}", start.elapsed().as_secs());
    if msg_count == 0 {
        println!("status: no_messages");
    } else {
        println!("status: ok");
    }

    if let Err(msg) = check_min_messages(msg_count, cli.min_messages) {
        eprintln!("{msg}");
        anyhow::bail!("insufficient messages captured");
    }

    if msg_count == 0 {
        eprintln!(
            "No messages received in {}s. The broker may be quiet with --from-end.\n\
             Try --from-end=false, a longer --max-seconds, or an exact --topic.",
            start.elapsed().as_secs()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_by_collector_fragment() {
        let topics = vec![
            "routeviews.chicago.65000.bmp_raw".into(),
            "routeviews.linx.8714.bmp_raw".into(),
            "routeviews.sg.64050.bmp_raw".into(),
        ];
        let filtered = apply_filters(topics, Some("chicago"), None);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("chicago"));
    }

    #[test]
    fn test_filter_by_collector_case_insensitive() {
        let topics = vec![
            "routeviews.LINX.8714.bmp_raw".into(),
            "routeviews.sg.64050.bmp_raw".into(),
        ];
        let filtered = apply_filters(topics, Some("linx"), None);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("LINX"));
    }

    #[test]
    fn test_filter_by_asn_suffix() {
        let topics = vec![
            "routeviews.chicago.65000.bmp_raw".into(),
            "routeviews.linx.8714.bmp_raw".into(),
            "routeviews.sg.64050.bmp_raw".into(),
        ];
        let filtered = apply_filters(topics, None, Some("8714"));
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].ends_with(".8714.bmp_raw"));
    }

    #[test]
    fn test_filter_combined_collector_and_asn() {
        let topics = vec![
            "routeviews.chicago.65000.bmp_raw".into(),
            "routeviews.chicago.13335.bmp_raw".into(),
            "routeviews.linx.13335.bmp_raw".into(),
        ];
        let filtered = apply_filters(topics, Some("chicago"), Some("13335"));
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("chicago"));
        assert!(filtered[0].ends_with(".13335.bmp_raw"));
    }

    #[test]
    fn test_filter_no_matches() {
        let topics = vec!["routeviews.chicago.65000.bmp_raw".into()];
        let filtered = apply_filters(topics, Some("nonexistent"), None);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_no_filters_returns_all() {
        let topics = vec!["routeviews.a.bmp_raw".into(), "routeviews.b.bmp_raw".into()];
        let filtered = apply_filters(topics.clone(), None, None);
        assert_eq!(filtered, topics);
    }

    #[test]
    fn test_min_messages_ok() {
        assert!(check_min_messages(10, 5).is_ok());
        assert!(check_min_messages(5, 5).is_ok());
        assert!(check_min_messages(100, 0).is_ok());
        assert!(check_min_messages(0, 0).is_ok());
    }

    #[test]
    fn test_min_messages_fails() {
        assert!(check_min_messages(0, 1).is_err());
        assert!(check_min_messages(3, 10).is_err());
    }
}
