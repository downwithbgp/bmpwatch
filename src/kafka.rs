use std::time::Duration;

use anyhow::{Context, Result};
use rdkafka::config::ClientConfig;
use rdkafka::consumer::BaseConsumer;
use rdkafka::consumer::Consumer;

/// Fetch topic names from a Kafka broker matching a regex pattern.
pub fn fetch_topics(broker: &str, pattern: &str) -> Result<Vec<String>> {
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

/// Post-filter topic list by collector fragment (case-insensitive substring)
/// and/or ASN suffix (`.ASN.bmp_raw`).
pub fn apply_filters(
    topics: Vec<String>,
    collector: Option<&str>,
    asn: Option<&str>,
) -> Vec<String> {
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

/// Create a Kafka BaseConsumer with standard config for RouteViews.
pub fn create_consumer(broker: &str, group_id: &str, from_end: bool) -> Result<BaseConsumer> {
    let offset_reset = if from_end { "latest" } else { "earliest" };
    ClientConfig::new()
        .set("bootstrap.servers", broker)
        .set("group.id", group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", offset_reset)
        .set("session.timeout.ms", "10000")
        .create()
        .context("Failed to create Kafka consumer")
}
