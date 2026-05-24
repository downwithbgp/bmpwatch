# BMPWatch

BMPWatch is a diagnostic and learning tool for BGP Monitoring Protocol
streams. It validates RFC 7854 BMP framing, unwraps OpenBMP `OBMP` payloads,
and provides a live TUI dashboard with RPKI origin validation. It is useful
for network operators, students, parser development, lab testing, capture
sanity checks, and BMP literacy.

### Scope

BMPWatch is **not** an observability platform or a replacement for existing
routing analytics tools. It does not perform deep BGP UPDATE semantic
validation or native PCAP/TCP reassembly. The focus is BMP stream inspection,
framing validation, and an interactive learning dashboard.

This is an **experimental v0.1 release**. It is not production monitoring.
It is good for learning, exploration, demos, and capture sanity checks.

## Supported inputs

| Input | Format | How |
|-------|--------|-----|
| `.rawbmp` | Raw RFC 7854 BMP frames | `--format auto` (default) or `--format raw-bmp` |
| `.bmpd` | BMPWatch local capture container | `--format auto` (default) or `--format bmpd` |
| Public Kafka (e.g. RouteViews) | Live `*.bmp_raw` topics | `record_openbmp_kafka` saves as `.bmpd` |
| OpenBMP `OBMP` wrapper | Upstream wrapper inside Kafka payloads | Stripped automatically by `.bmpd` parser |
| BGPReader / MRT / PCAP | N/A | Comparison or external extraction only |

> `.bmpd` is BMPWatch's local capture container. It is not an OpenBMP
> standard. Its records may contain raw BMP frames or upstream OpenBMP
> `OBMP`-wrapped payloads.

## Purpose

BMPWatch scans binary BMP input and produces:

- **dashboard** — live TUI for public BMP streams with RPKI validation (the default mode)
- **inspect** — human-readable summary of file contents, message counts, peer state
- **lint** — machine-oriented finding output with severity levels and exit codes
- **dump --jsonl** — one JSON object per message for debugging and automation

## Quick start

### Offline smoke test (no network required)

```sh
# Unit test
cargo test obmp_reader::tests::test_committed_fixture_two_openbmp_records

# Inspect a committed fixture
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/openbmp-two-records.bmpd --summary-json
```

Expected: `malformed_messages=0`, `total_messages=2`, `container_records=2`.

### Live dashboard

The default mode — opens the TUI stream browser. No arguments needed:

```sh
cargo run --bin bmpwatch
```

With topic flags:

```sh
cargo run --bin bmpwatch -- --collector chicago --asn 13335
```

### Record and inspect a live capture (network required)

1. **Record a `.bmpd` capture** from a public Kafka source:
   ```sh
   cargo run --bin record_openbmp_kafka -- \
     --topic-regex '^routeviews.*\.bmp_raw$' \
     --out samples/capture.bmpd \
     --max-messages 100
   ```

2. **Inspect** (format auto-detected):
   ```sh
   cargo run --bin bmpwatch -- \
     inspect samples/capture.bmpd --summary-json
   ```

3. **Understand the layers** (see [Terminology](#terminology)):
   - `.bmpd` = BMPWatch capture container (`BMPDOPENBMP1\n`)
   - `OBMP` = upstream OpenBMP wrapper inside Kafka payloads
   - Inner frame = RFC 7854 BMP message

### Choose a live feed

Before recording, discover available feeds from the broker. Commands below
assume `cargo install --path .` has been run; use `cargo run --bin` if
building from source without installing.

```sh
# List all topics from a collector group (case-insensitive)
record_openbmp_kafka --list-topics --collector chicago

# List topics for a specific peer ASN
record_openbmp_kafka --list-topics --asn 13335

# Narrow to one collector + one ASN
record_openbmp_kafka --list-topics --collector chicago --asn 13335

# JSON output (scriptable)
record_openbmp_kafka --list-topics-json --collector nwax
```

`--collector` filters topic names by a case-insensitive fragment (collector
or router group name). `--asn` filters topics ending with `.<ASN>.bmp_raw`.
Both apply after regex matching and before the `--topic-limit` safety guard,
which refuses subscriptions matching more than 20 topics by default.

> **v0.1 source support**: the built-in Kafka source targets RouteViews'
> public broker at `stream.routeviews.org:9092`. Future versions may add
> additional live sources. The `.bmpd` container format and `.rawbmp` file
> input are source-agnostic.

### Live capture smoke test (network required)

The 2-step workflow to verify end-to-end health against a live public BMP source:

```sh
# Step 1: capture 100 messages
cargo run --bin record_openbmp_kafka -- \
  --out samples/smoke.bmpd --max-messages 100 --min-messages 1

# Step 2: inspect, check for malformed
cargo run --bin bmpwatch -- \
  inspect samples/smoke.bmpd --summary-json
```

Pass condition: `malformed_messages == 0`. Warnings from mid-stream capture
(typically `stream_order_warnings`) are expected observations, not failures.
Unlike the offline smoke test, this requires network access to
`stream.routeviews.org:9092` and may produce stream-order warnings.
A useful capture should show `status: ok` and `messages_written > 0` in
the recorder summary; `status: no_messages` means the file only contains
the `.bmpd` container magic and should not be used as a validation sample.
The capture step uses `--min-messages 1` to exit non-zero on magic-only
captures.
See [Real-sample validation](docs/real-sample-validation.md) for an example
of expected output with 106 warnings / 0 parse errors.

The `findings_buckets` field in `--summary-json` separates counts into:
- `parse_errors` — parser/frame-level issues (version, truncation, unknown type)
- `stream_order_warnings` — protocol/stream-order observations (RM before PeerUp, timestamp regression)
- `other_findings` — anything not in the above categories

## Installation

```sh
cargo install --path .
```

## Usage

### Input format

`--format` defaults to `auto`, which reads the first bytes of the file:

| First bytes | Detected format |
|-------------|-----------------|
| `BMPDOPENBMP1\n` | `bmpd` |
| `0x03` | `raw-bmp` |
| Unknown / empty / short | `raw-bmp` (diagnostic fallback — let the parser report errors) |

> Auto-detection only recognizes valid `.bmpd` magic (`BMPDOPENBMP1\n`)
> and raw BMP (`0x03` first byte). Use explicit `--format bmpd` for
> container-level diagnostics on corrupted `.bmpd` files.

Explicit `--format raw-bmp` or `--format bmpd` overrides auto-detection
and can intentionally produce malformed/error output if the wrong format is
forced (useful for debugging or format-level misuse testing).

> The dashboard connects to a live Kafka broker. `--format` is only used in
> file replay mode (`bmpwatch <file>`).

### inspect

```sh
bmpwatch inspect path/to/bmp-data.rawbmp
```

Outputs file metadata, message type counts, per-peer statistics, active peer count,
top peers by route-monitoring messages, and a findings summary.

`--max-peers <N>` (default 10) controls how many peers appear in the peer
inventory table and the "Top peers" section. `--max-peers 0` suppresses
both peer-list sections while keeping aggregate counts (`peers_observed`,
`active_peers`). In `--summary-json`, the `peers` array is absent when
`--max-peers` is 0.

With `--summary-json`, outputs machine-readable totals. For `.bmpd` files,
a `container` section distinguishes the capture wrapper from the payload types:

```json
{
  "file": "samples/routeviews-broad-100.bmpd",
  "format": "BMPWatch container",
  "size_bytes": 27630,
  "total_messages": 100,
  "malformed_messages": 0,
  "bgp_elem_count": 29,
  "by_type": { "RouteMonitoring": 100 },
  "peers_observed": 18,
  "active_peers": 0,
  "info_count": 0,
  "warn_count": 106,
  "error_count": 0,
  "findings_truncated": false,
  "findings_dropped_count": 0,
  "findings_buckets": {
    "parse_errors": 0,
    "stream_order_warnings": 106,
    "other_findings": 0
  },
  "session_lifecycle": {
    "peers_observed": 18,
    "active_peers": 0,
    "route_monitoring_messages": 100,
    "peer_up_messages": 0,
    "peer_down_messages": 0,
    "rm_before_peer_up_warnings": 100
  },
  "peers": [
    {
      "peer_asn": 3303,
      "peer_ip": "2001:07f8:0000:0000:0000:0ce7:0000:0002",
      "active": false,
      "rm_count": 42,
      "up_count": 0,
      "down_count": 0
    }
  ],
  "container": {
    "container_records": 100,
    "raw_bmp_payloads": 0,
    "openbmp_wrapped_payloads": 100,
    "openbmp_metadata": {
      "collector": "bmp-01",
      "router": "namex.fco",
      "router_ip": "185.33.111.234"
    }
  }
}
```

The `container` section is intentionally absent for `raw-bmp` input, which
has no `.bmpd` record layer. `openbmp_metadata` appears only when records
contain an OpenBMP `OBMP` wrapper with populated fields; it is not guaranteed
for all `.bmpd` files. Zero-value container fields and absent metadata keys
are omitted from the output to keep the JSON compact.

The `inspect` text output shows the same metadata between the peer list and
findings summary:

```
OpenBMP metadata:
  Collector:  bmp-01
  Router:     namex.fco
  Router IP:  185.33.111.234
```

`container` counters (present for `.bmpd` input, absent for `raw-bmp`):

| Counter | Meaning |
|---------|---------|
| `container_records` | Total `.bmpd` records in the file |
| `raw_bmp_payloads` | Records with a raw RFC 7854 BMP frame (starts `0x03`) |
| `openbmp_wrapped_payloads` | Records with an `OBMP` wrapper (mutually exclusive with `raw_bmp_payloads` per record) |
| `unrecognized_payloads` | Records that are neither raw BMP nor `OBMP` |
| `openbmp_unwrap_errors` | `OBMP` wrapper header parsing failures |
| `inner_bmp_parse_errors` | `OBMP` unwrap succeeded but inner BMP frame parsing failed (mutually exclusive with `openbmp_unwrap_errors`) |

### lint

```sh
bmpwatch lint path/to/bmp-data.rawbmp
```

Emits one finding per line with severity, rule name, offset, and peer context. Exit codes:

| Exit | Meaning                  |
|------|--------------------------|
| 0    | Clean or info only       |
| 1    | Warnings present         |
| 2    | Errors or malformed frames |

### dump

```sh
bmpwatch dump path/to/bmp-data.rawbmp --jsonl
```

Emits one JSON object per observed BMP message including offset, type, peer identity,
timestamp, parse status, and associated findings. Per-message detail objects
(`tlv_info`, `stats_info`, `peer_down_info`) appear only when the message type
carries that information.

### replay (file mode)

```sh
bmpwatch <file> [--window-messages <n>] [--interval-ms <n>] [--format auto|raw-bmp|bmpd]
```

Replays a `.bmpd` or `.rawbmp` capture file as a rolling window stream, emitting
periodic JSON summary lines to stdout.

```json
{"window_messages":10,"total_seen":2,"elapsed_ms":0,"by_type":{"PeerUpNotification":1,"RouteMonitoring":1},"peers_observed":1,...}
```

### dashboard (default)

```sh
bmpwatch [--broker <host>] [--topic <exact>] [--collector <frag>] [--asn <n>]
```

The default mode — run `bmpwatch` with no arguments to open the live TUI.
Connects to a public BMP Kafka broker and shows a search-powered stream
browser. Type to filter across ASN, name, collector, or topic. Arrow keys
navigate, Enter connects. Supports the `--broker` flag for future source
adapters.

In the dashboard:
- **Live message log** — scrolling view of BMP messages with timestamps, prefix
  announcements/withdrawals (green/red), RPKI validation badges (VAL/ASN/LEN/NF),
  and compact AS paths (deduplicated, truncated, origin AS color-coded by RPKI)
- **Prefix Flaps** — prefixes grouped by origin ASN, sorted by churn frequency,
  with AS names resolved via bundled seed data, live WHOIS lookups, and
  session-persistent cache (`~/.cache/bmpwatch/as_names_cache.bin`)
- **RPKI validation** — downloads ROAs from Cloudflare RTR server on startup
  (~430K VRPs, cached 6 hours to `~/.cache/bmpwatch/rpki_cache.bin`). Invalid
  prefixes show the expected ASN ("should be AS64496") or max prefix length
- **Status bar** — RPKI counts (VAL/INV/NF), cumulative messages, rate,
  findings status, and keybindings
- Pause (`p`) freezes the display and shows a yellow header — log stays visible for study

Keys: `q`/`Esc` quit, `b` back to stream browser, `p` pause/resume.

## Detected issues

BMPWatch checks for:

| Rule                               | Severity | Description                                         |
|------------------------------------|----------|-----------------------------------------------------|
| `invalid_bmp_version`              | ERR      | BMP version is not 3                                |
| `truncated_frame`                  | ERR      | Frame header declares length beyond available data  |
| `unknown_bmp_type`                 | WARN     | BMP message type outside 0–6                        |
| `parse_error`                      | ERR      | BGPKIT Parser could not parse the message            |
| `route_monitoring_before_peer_up`  | WARN     | RM message before any Peer Up for that peer         |
| `duplicate_peer_up`                | WARN     | Peer Up for a peer that is already active           |
| `peer_down_without_peer_up`        | WARN     | Peer Down for a peer that was not active            |
| `timestamp_regression`             | WARN     | Timestamp went backwards for a given peer           |

## Input model

How raw BMP frames, BMPWatch containers, upstream wrappers, and external
data sources relate to each other.

| Layer | Format | Characteristic |
|-------|--------|----------------|
| Raw BMP frame | `.rawbmp` | Concatenated RFC 7854 BMP messages. First byte `0x03`. Direct TCP byte stream capture. |
| BMPWatch container | `.bmpd` | Local capture wrapper: `BMPDOPENBMP1\n` magic + u32 BE length-prefixed records. Not an OpenBMP standard. |
| Upstream wrapper | `OBMP` | OpenBMP header inside RouteViews Kafka `*.bmp_raw` payloads. Stripped automatically when reading `.bmpd` records. |
| RouteViews Kafka | N/A | `record_openbmp_kafka` saves payloads into `.bmpd`. Payloads are typically `OBMP`-wrapped. |
| MRT / BGPReader | N/A | Not BMP. BGPReader output is decoded BGP events, not raw BMP frames. Comparison-only; no direct parsing. |
| PCAP | N/A | BMP over TCP requires stream reassembly. External-tool workflow documented; native support deferred. |

Each `.bmpd` record contains one raw BMP frame or one `OBMP`-wrapped raw
BMP frame. BMPWatch's pipeline unwraps `OBMP` (if present) and parses the
inner RFC 7854 frame.

## Terminology

| Term | Meaning |
|------|---------|
| `.bmpd` | BMPWatch's local capture container format. `BMPDOPENBMP1\n` magic + `u32` BE length-prefixed records. |
| `OBMP` | OpenBMP upstream wrapper inside each RouteViews Kafka `*.bmp_raw` payload. Stripped automatically before BMP frame parsing. |
| Inner frame | The RFC 7854 BMP message (common header + per-peer header + body). This is what BMPWatch's parser operates on. |

## Privacy

BMP captures contain live BGP routing data. This data can reveal network
relationships, upstream providers, and routing policies. Do not commit
private BMP captures to public repositories.

## Limitations

BMPWatch evaluates observed ordering within the input file. If a file starts
mid-session, warnings like `route_monitoring_before_peer_up` may indicate an
incomplete capture rather than a broken BMP feed. Warnings from live captures
are expected — they reflect real-world stream ordering, not parser
failures.

Frame-level validation checks the BMP common header (version, length, type) but
does not perform deep BGP attribute validation beyond what BGPKIT Parser provides.
Initiation and Termination message information TLVs (sysDescr, sysName,
termination reason) are decoded per RFC 7854 / IANA BMP Parameters; unknown TLVs
are displayed safely by type number. Peer Down reason codes are decoded from a
separate RFC 7854 / IANA registry. Basic Stats Report type/value entries are
decoded for diagnostics; BMPWatch is not a time-series analytics tool or
Prometheus exporter.

Core inspection supports `.rawbmp` and `.bmpd` formats. Other formats
(PCAP, MRT/BGPReader, `.bmpr`, compressed `.bz2`/`.gz`) are not native
inputs. RouteViews Kafka is supported through the separate
`record_openbmp_kafka` binary; native Kafka input inside core `bmpwatch`
is out of scope. TCP listener mode and streaming inputs are out of scope
for the MVP.

## Sources & capture

- [Data sources reference](docs/sources.md) — public BMP Kafka sources, file inputs
- [OpenBMP Kafka capture guide](docs/openbmp-kafka-capture.md) — connectivity testing with `nc`/`kcat`
- [Future issues](docs/future-issues.md) — planned features and their scope
- [Real-sample validation](docs/real-sample-validation.md) — verified capture + parse results

## Roadmap

Historical tags (`v0.1.*`) are checkpoints, not polished releases.

### A. Near-term (good next BMPWatch work)

- Better RFC 7854 message summaries and TLV display
- More synthetic fixtures for edge-case coverage
- Better peer/session lifecycle summaries
- Additional real RouteViews sample validation notes
- Local FRR BGP lab verified; FRR BMP output blocked on tested images
  (see [FRR local BMP lab](docs/frr-local-bmp-lab.md))

### B. Useful later

- PCAP/PCAPNG via external extraction first; native support deferred
  (see [PCAP support note](docs/pcap-support.md))
- BMPWatch Observatory: public live learning UI for selected
  RouteViews / OpenBMP-derived telemetry streams
  (see [vision doc](docs/bmpwatch-observatory-vision.md))
- File replay (`bmpwatch <file>`) as a stepping stone to live Observatory
  (see [replay/watch design](docs/replay-watch-design.md))
- Standalone `OBMP` payload file support if real samples justify it
- Compressed input (`.bz2`, `.gz`)
- `.bmpr` capture format support
- RouteViews `bgpreader` PSV comparison tooling
- More output formats if there is a clear consumer

### C. Explicitly not now

- Deep BGP UPDATE semantic validation
- Full observability platform / storage backend
- Prometheus metrics / Parquet export
- Native Kafka input in core `bmpwatch`
- TCP listener mode
- RFC 8671 / 9069 / 9736 interpretation until base RFC 7854 behavior is mature

### Implemented and verified

- `record_openbmp_kafka.rs` — RouteViews Kafka recorder (100 msgs, 4s)
- `--format bmpd` — BMPWatch container with OpenBMP unwrap (100 msgs,
  18 peers, 0 malformed)
- `--format auto` — content-based format detection
- 8 lint rules, findings buckets, peer inventory, Initiation/Termination
  TLV decoding

## License

MIT
