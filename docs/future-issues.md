# Future Issues

Planned work items for post-MVP development. These are not implemented.

## 1. examples/record_openbmp_kafka.rs

**Status:** Implemented and verified (as `src/bin/record_openbmp_kafka.rs`)
**Scope:** Standalone binary, not integrated into `bmpdoctor` core CLI

RouteViews broad regex capture verified: 100 messages, 27,630 bytes,
4-second duration. See `docs/routeviews-kafka-verification.md`.

**Target broker:** `stream.routeviews.org:9092` — verified reachable.
**Historical broker:** CAIDA's `bmp.bgpstream.caida.org:9092` is
unreachable. See `docs/caida-kafka-verification.md`.

### Verified smoke command

```sh
cargo run --bin record_openbmp_kafka -- \
  --broker stream.routeviews.org:9092 \
  --topic-regex '^routeviews.*\.bmp_raw$' \
  --out samples/routeviews-broad-100.bmpd \
  --max-messages 100 \
  --max-seconds 60 \
  --from-end
```

Observed: messages_written=100, bytes_written=27630, duration_secs=4.

A Kafka consumer that connects to an OpenBMP broker and writes captured BMP
data to local files.

### Requirements

- Connect to a Kafka broker (configurable host/port)
- Subscribe to topics matching `^openbmp\.router--.+\.peer-as--.+\.bmp_raw`
- Consume messages as raw BMP frames (no OpenBMP wrapper at the Kafka layer)
- Write frames to a local `.bmpd` file (BMPDoctor container format with
  `BMPDOPENBMP1` magic + `u32` BE length prefix per frame), preserving
  byte-for-byte fidelity
- Optional: rotate output files by size or time
- Optional: track consumer offsets for resumability

### Dependencies

- `rdkafka` crate (librdkafka bindings)

### Example sketch

```rust
// examples/record_openbmp_kafka.rs
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::ClientConfig;
use rdkafka::Message;

fn main() {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", "bmp.bgpstream.caida.org:9092")
        .set("group.id", "bmpdoctor-capture")
        .create()
        .unwrap();

    consumer.subscribe(&["^openbmp\\.router--.+\\.peer-as--.+\\.bmp_raw"]).unwrap();

    for msg in &consumer {
        if let Some(payload) = msg.payload() {
            // write payload to output file
        }
    }
}
```

### Out of scope

- Kafka input is NOT added to the `bmpdoctor` CLI as a subcommand or flag
- No Kafka producer or relay functionality
- No integration with `bmpdoctor inspect/lint/dump` beyond writing local files

---

## 2. --format bmpd

**Status:** Implemented and verified
**Scope:** `--format bmpd` flag on `inspect`/`lint`/`dump`

Parses `.bmpd` container files (`BMPDOPENBMP1\n` magic + `u32` BE length-
prefixed records). Detects raw BMP (first byte `0x03`) and OpenBMP-wrapped
payloads (first bytes `OBMP`). OpenBMP unwrap uses bgpkit-parser's
`parse_openbmp_header` to strip the wrapper, then passes the inner RFC 7854
BMP frame through the existing parser pipeline.

Verified against real RouteViews capture: 100 messages, 18 peers, 29 BGP
elements, 0 malformed.

### Future work

- Richer display of OpenBMP metadata (collector ID, router IP, admin ID)
  during `inspect` output
- Live-stream diagnostics (incremental counter tracking between repeated
  `.bmpd` file reads)
- Richer format detection: standalone upstream `OBMP` payload files,
  surfacing detected format in more output modes, exposing detection
  as a library API. (Basic `.bmpd` container detection via
  `BMPDOPENBMP1\n` is already implemented in `--format auto`.)

---

## 3. Compressed input support (`.bz2`, `.gz`)

**Status:** Not started  
**Scope:** Decompress before frame scanning

Add transparent decompression when the file extension or magic bytes indicate
bzip2 or gzip compression. Use `bzip2` and `flate2` crates.

---

## 4. Public BMP fixture corpus

**Status:** Not started  
**Scope:** Testing infrastructure

Collect or generate a set of BMP fixture files covering:
- Valid sessions (init, peer up, route monitoring, peer down)
- Edge cases (truncated frames, invalid version, mid-stream start)
- Multiple peers, timestamp regression, malformed BGP updates
- IPv4 and IPv6 peer addresses
- All BMP message types (0–6)

Store in `tests/fixtures/` with metadata describing expected behavior.

---

## 5. PCAP / PCAPNG support

**Status:** Deferred  
**Scope:** `--format pcap` for BMP-over-TCP packet captures

BMP runs over TCP, so PCAP support requires TCP stream reassembly before
BMP frame parsing. This adds significant complexity (segment reordering,
retransmission handling, flow selection). See
[docs/pcap-support.md](pcap-support.md) for a full design assessment.

**Near-term workflow:** Extract BMP TCP stream payloads to `.rawbmp` using
external tools (`tshark`, `tcpflow`), then run BMPDoctor normally.
