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
    about = "Record RouteViews Kafka BMP messages to a local .obmp file"
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

    #[arg(long, help = "List matching topics and exit (no recording)")]
    list_topics: bool,

    #[arg(
        long,
        default_value_t = DEFAULT_TOPIC_LIMIT,
        help = "Refuse to subscribe if regex matches more than N topics"
    )]
    topic_limit: usize,
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(ref exact) = cli.topic {
        // Exact topic mode — skip regex matching
        if cli.list_topics {
            println!("{exact}");
            return Ok(());
        }
        run_record(&cli, vec![exact.clone()])
    } else {
        let matched = fetch_topics(&cli.broker, &cli.topic_regex)?;

        if matched.is_empty() {
            anyhow::bail!(
                "No topics matched regex '{}'. Check --broker and --topic-regex.",
                cli.topic_regex
            );
        }

        if cli.list_topics {
            for t in &matched {
                println!("{t}");
            }
            eprintln!("{} topics matched", matched.len());
            return Ok(());
        }

        if matched.len() > cli.topic_limit {
            let show = matched.len().min(3);
            let sample: Vec<_> = matched.iter().take(show).collect();
            anyhow::bail!(
                "Regex matched {} topics but --topic-limit is {}.\n\
                 First {} topic(s):\n  {}\n\n\
                 Choose one of:\n  \
                 --topic <exact-topic>       pick a single topic\n  \
                 --topic-limit <N>           raise the limit\n  \
                 --list-topics              see all matching topics",
                matched.len(),
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
            matched.len(),
            cli.topic_limit,
        );
        run_record(&cli, matched)
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

    // Summary to stdout
    println!("broker: {}", cli.broker);
    println!("topic/s: {}", topics.join(", "));
    println!("messages_written: {msg_count}");
    println!("bytes_written: {byte_count}");
    println!("output_path: {}", cli.out.display());
    println!("duration_secs: {}", start.elapsed().as_secs());

    // Zero-message diagnostic
    if msg_count == 0 {
        eprintln!(
            "No messages received in {}s. The broker may be quiet with --from-end.\n\
             Try --from-end=false, a longer --max-seconds, or an exact --topic.",
            start.elapsed().as_secs()
        );
    }

    Ok(())
}
