# OpenBMP Kafka Capture

How to verify reachability and test capture from an OpenBMP Kafka broker
before integrating into BMPWatch.

## Preferred broker: RouteViews

`stream.routeviews.org:9092` is the verified working broker for
BMPWatch integration. See
[RouteViews Kafka verification](routeviews-kafka-verification.md)
for the full test log.

**Topic regex:** `^route-?views\..*\.bmp_raw$`

**Warning:** Topic naming is not perfectly uniform. Both `routeviews.`
and `route-views.` prefixes have been observed. Consumers must handle
both variants.

## Historical: CAIDA BGPStream broker

`bmp.bgpstream.caida.org:9092` was tested in May 2026 and found
unreachable from the developer's network. It is preserved in
documentation for historical reference only. See
[CAIDA Kafka verification](caida-kafka-verification.md).

## Prerequisites

Install `kcat` (formerly `kafkacat`):

```sh
# macOS
brew install kcat

# Debian/Ubuntu
sudo apt-get install kafkacat
```

## 1. Broker reachability test

```sh
nc -vz stream.routeviews.org 9092
```

## 2. Discover topics (recorder-based)

Use the recorder's built-in topic discovery:

```sh
# List all topics from a collector group
cargo run -p record_openbmp_kafka -- --list-topics --collector chicago

# List by peer ASN
cargo run -p record_openbmp_kafka -- --list-topics --asn 13335

# Narrow to one collector + one ASN
cargo run -p record_openbmp_kafka -- --list-topics --collector chicago --asn 13335

# JSON output for scripts
cargo run -p record_openbmp_kafka -- --list-topics-json --collector nwax
```

`--collector` filters by a case-insensitive fragment in the topic name
(collector or router group). `--asn` filters topics ending with
`.<ASN>.bmp_raw`. Both apply after regex matching and before the
`--topic-limit` safety guard.

### Low-level discovery (kcat)

```sh
kcat -b stream.routeviews.org:9092 -L
```

### Discover then record

```sh
# Step 1: find a topic
cargo run -p record_openbmp_kafka -- --list-topics --asn 13335

# Step 2: record from that exact topic
cargo run -p record_openbmp_kafka -- \
  --topic routeviews.chicago.13335.bmp_raw \
  --out samples/chicago-13335.bmpd \
  --max-messages 100
```

Look for topics matching `^route-?views\..*\.bmp_raw$`.

## 3. Consume a single topic (one peer session)

Pick a topic from the listing and subscribe:

```sh
kcat -b stream.routeviews.org:9092 \
  -t routeviews.sg.64050.bmp_raw \
  -C -o beginning -c 10
```

Flags:
- `-C`: consumer mode
- `-o beginning`: start from earliest available offset
- `-c 10`: consume 10 messages only

Each message printed will be binary BMP frame data. Pipe through `xxd` or
`hexdump -C` for inspection:

```sh
kcat -b stream.routeviews.org:9092 \
  -t routeviews.sg.64050.bmp_raw \
  -C -o beginning -c 1 | xxd | head -20
```

## 4. Consume multiple peer sessions (regex subscribe)

`kcat` does not support topic regex natively, but you can list topics and
spawn one consumer per topic, or use `kafka-console-consumer` with
`--whitelist`:

```sh
kafka-console-consumer \
  --bootstrap-server stream.routeviews.org:9092 \
  --whitelist 'routeviews\..*\.bmp_raw' \
  --from-beginning \
  --max-messages 100
```

## 5. Capture to a local .bmpd file (using the recorder binary)

The `record_openbmp_kafka` binary handles Kafka connection and writes
BMP messages to a `.bmpd` file:

```sh
# Exact topic
cargo run -p record_openbmp_kafka -- \
  --topic routeviews.nwax.13335.bmp_raw \
  --out samples/nwax-sample.bmpd \
  --max-messages 100

# Topic regex (subscribes to all matching topics)
cargo run -p record_openbmp_kafka -- \
  --topic-regex '^route-?views\..*\.bmp_raw$' \
  --out samples/routeviews-sample.bmpd \
  --max-messages 10 \
  --max-seconds 30
```

The `.bmpd` file uses the `BMPDOPENBMP1\n` magic header followed by
repeated `u32` BE length + payload frames. Format is auto-detected by
BMPWatch; use `inspect`, `lint`, or `dump` directly.

**Prerequisite:** librdkafka must be installed:
```sh
brew install librdkafka          # macOS
sudo apt-get install librdkafka-dev  # Debian/Ubuntu
```

Then inspect with BMPWatch (auto-detected, no `--format` flag needed):

```sh
bmpwatch inspect samples/nwax-sample.bmpd --summary-json
bmpwatch dump samples/nwax-sample.bmpd --jsonl | head -5
```

## Notes

- Messages arrive as raw BMP frames (common header + payload). RouteViews
  Kafka payloads are typically OpenBMP `OBMP`-wrapped.
- The `record_openbmp_kafka` binary (at `record_openbmp_kafka/src/main.rs`)
  adds the `BMPDOPENBMP1` + `u32` BE length wrapper to each frame on write.
- The broker may throttle or close connections that consume too fast.
