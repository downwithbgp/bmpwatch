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

#[derive(Parser)]
#[command(
    name = "record_openbmp_kafka",
    version,
    about = "Record RouteViews Kafka BMP messages to a local .obmp file"
)]
struct Cli {
    #[arg(long, default_value = "stream.routeviews.org:9092")]
    broker: String,

    #[arg(long, help = "Exact Kafka topic name (overrides --topic-regex)")]
    topic: Option<String>,

    #[arg(
        long,
        default_value = "^route-?views\\..*\\.bmp_raw$",
        help = "Regex to match topic names (ignored if --topic is set)"
    )]
    topic_regex: String,

    #[arg(
        long,
        default_value = "samples/routeviews-sample.obmp",
        help = "Output .obmp file path"
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl-C handler")?;

    let topics = if let Some(ref exact) = cli.topic {
        vec![exact.clone()]
    } else {
        let consumer: BaseConsumer = ClientConfig::new()
            .set("bootstrap.servers", &cli.broker)
            .set("group.id", "bmpdoctor-recorder-metadata")
            .create()
            .context("Failed to create metadata consumer")?;

        let metadata = consumer
            .fetch_metadata(None, Duration::from_secs(10))
            .context("Failed to fetch topic metadata")?;

        let regex = regex::Regex::new(&cli.topic_regex)
            .with_context(|| format!("Invalid topic regex: {}", cli.topic_regex))?;

        let mut matched: Vec<String> = metadata
            .topics()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| regex.is_match(name))
            .collect();
        matched.sort();

        if matched.is_empty() {
            anyhow::bail!(
                "No topics matched regex '{}'. Check --broker and --topic-regex.",
                cli.topic_regex
            );
        }

        eprintln!(
            "Regex '{}' matched {} topics",
            cli.topic_regex,
            matched.len()
        );
        for t in &matched {
            eprintln!("  {t}");
        }

        matched
    };

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

    let mut writer = ObmpWriter::create(&cli.out).context("Failed to create output .obmp file")?;

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

    if cli.topic.is_none() {
        eprintln!(
            "NOTE: .obmp file written. Use --topic-regex with the future --format openbmp-len parser."
        );
    }

    Ok(())
}
