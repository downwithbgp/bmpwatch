# BMPDoctor

BMPDoctor is a file-first diagnostic tool for BGP Monitoring Protocol data. It uses
[BGPKIT Parser](https://github.com/bgpkit/bgpkit-parser) for protocol parsing and
focuses on stream/session health: frame validity, peer lifecycle, timestamp sanity,
malformed messages, per-peer summaries, and reproducible diagnostics.

## Purpose

BMPDoctor scans binary BMP frame files and produces:

- **inspect** — human-readable summary of file contents, message counts, peer state
- **lint** — machine-oriented finding output with severity levels and exit codes
- **dump --jsonl** — one JSON object per message for debugging and automation

## Happy path (RouteViews live data)

The recommended workflow from zero to verified results:

1. **Verify Kafka reachability** (one-time):
   ```sh
   nc -vz stream.routeviews.org 9092
   kcat -b stream.routeviews.org:9092 -L
   ```

2. **Record a `.obmp` capture**:
   ```sh
   cargo run --bin record_openbmp_kafka -- \
     --topic-regex '^routeviews.*\.bmp_raw$' \
     --out samples/capture.obmp \
     --max-messages 100
   ```

3. **Inspect** (format auto-detected):
   ```sh
   cargo run --bin bmpdoctor -- \
     inspect samples/capture.obmp --summary-json
   ```

4. **Understand the layers** (see [Terminology](#terminology)):
   - `.obmp` = BMPDoctor capture container (`BMPDOPENBMP1\n`)
   - `OBMP` = upstream OpenBMP wrapper inside Kafka payloads
   - Inner frame = RFC 7854 BMP message

### Known-good smoke test

```sh
cargo test obmp_reader::tests::test_committed_fixture_two_openbmp_records
```

Verifies that the committed 2-record `.obmp` fixture (PeerUp + RouteMonitoring,
both OpenBMP-wrapped) parses correctly without network dependency.

### Offline smoke test (committed fixture, no network)

```sh
cargo run --bin bmpdoctor -- \
  inspect tests/fixtures/openbmp-two-records.obmp --summary-json
```

Expected: `malformed_messages=0`, `total_messages=2`, `container.container_records=2`,
`container.openbmp_wrapped_payloads=2`, `container.openbmp_metadata` present.

Shell one-liner (exits 0 on pass, 1 on failure):

```sh
cargo run --bin bmpdoctor -- \
  inspect tests/fixtures/openbmp-two-records.obmp --summary-json \
  | python3 -c "
import json,sys; d=json.load(sys.stdin);
ok = d['malformed_messages']==0 and d['total_messages']==2 \
     and d['container']['container_records']==2 \
     and d['container']['openbmp_wrapped_payloads']==2 \
     and 'openbmp_metadata' in d['container'];
print('PASS' if ok else 'FAIL'); sys.exit(0 if ok else 1)
"
```

Also available as a unit test:

```sh
cargo test obmp_reader::tests::test_committed_fixture_two_openbmp_records
```

Verifies the same fixture parses correctly without network dependency.

### Choose a RouteViews feed

Before recording, discover available feeds from the broker:

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

### RouteViews smoke test (live capture, network required)

The 2-step workflow to verify end-to-end health against real RouteViews data:

```sh
# Step 1: capture 100 messages
cargo run --bin record_openbmp_kafka -- \
  --out samples/smoke.obmp --max-messages 100 --min-messages 1

# Step 2: inspect, check for malformed
cargo run --bin bmpdoctor -- \
  inspect samples/smoke.obmp --summary-json
```

Pass condition: `malformed_messages == 0`. Warnings from mid-stream capture
(typically `stream_order_warnings`) are expected observations, not failures.
Unlike the offline smoke test, this requires network access to
`stream.routeviews.org:9092` and may produce stream-order warnings.
A useful capture should show `status: ok` and `messages_written > 0` in
the recorder summary; `status: no_messages` means the file only contains
the `.obmp` container magic and should not be used as a validation sample.
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
| `BMPDOPENBMP1\n` | `openbmp-len` |
| `0x03` | `raw-bmp` |
| Unknown / empty / short | `raw-bmp` (diagnostic fallback — let the parser report errors) |

Explicit `--format raw-bmp` or `--format openbmp-len` overrides auto-detection
and can intentionally produce malformed/error output if the wrong format is
forced (useful for debugging or format-level misuse testing).

### inspect

```sh
bmpdoctor inspect path/to/bmp-data.rawbmp
```

Outputs file metadata, message type counts, per-peer statistics, active peer count,
top peers by route-monitoring messages, and a findings summary.

`--max-peers <N>` (default 10) controls how many peers appear in the peer
inventory table and the "Top peers" section. `--max-peers 0` suppresses
both peer-list sections while keeping aggregate counts (`peers_observed`,
`active_peers`). In `--summary-json`, the `peers` array is absent when
`--max-peers` is 0.

With `--summary-json`, outputs machine-readable totals. For `.obmp` files,
a `container` section distinguishes the capture wrapper from the payload types:

```json
{
  "file": "samples/routeviews-broad-100.obmp",
  "format": "OpenBMP length-delimited",
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
has no `.obmp` record layer. `openbmp_metadata` appears only when records
contain an OpenBMP `OBMP` wrapper with populated fields; it is not guaranteed
for all `.obmp` files. Zero-value container fields and absent metadata keys
are omitted from the output to keep the JSON compact.

The `inspect` text output shows the same metadata between the peer list and
findings summary:

```
OpenBMP metadata:
  Collector:  bmp-01
  Router:     namex.fco
  Router IP:  185.33.111.234
```

`container` counters (present for `.obmp` input, absent for `raw-bmp`):

| Counter | Meaning |
|---------|---------|
| `container_records` | Total `.obmp` records in the file |
| `raw_bmp_payloads` | Records with a raw RFC 7854 BMP frame (starts `0x03`) |
| `openbmp_wrapped_payloads` | Records with an `OBMP` wrapper (mutually exclusive with `raw_bmp_payloads` per record) |
| `unrecognized_payloads` | Records that are neither raw BMP nor `OBMP` |
| `openbmp_unwrap_errors` | `OBMP` wrapper header parsing failures |
| `inner_bmp_parse_errors` | `OBMP` unwrap succeeded but inner BMP frame parsing failed (mutually exclusive with `openbmp_unwrap_errors`) |

### lint

```sh
bmpdoctor lint path/to/bmp-data.rawbmp
```

Emits one finding per line with severity, rule name, offset, and peer context. Exit codes:

| Exit | Meaning                  |
|------|--------------------------|
| 0    | Clean or info only       |
| 1    | Warnings present         |
| 2    | Errors or malformed frames |

### dump

```sh
bmpdoctor dump path/to/bmp-data.rawbmp --jsonl
```

Emits one JSON object per observed BMP message including offset, type, peer identity,
timestamp, parse status, and associated findings.

## Detected issues

BMPDoctor checks for:

| Rule                               | Severity | Description                                         |
|------------------------------------|----------|-----------------------------------------------------|
| `invalid_bmp_version`              | ERR      | BMP version is not 3                                |
| `truncated_frame`                  | ERR      | Frame header declares length beyond available data  |
| `unknown_bmp_type`                 | WARN     | BMP message type outside 0–6                        |
| `parse_error`                      | ERR      | BGPKIT Parser cannot parse the message              |
| `route_monitoring_before_peer_up`  | WARN     | RM message before any Peer Up for that peer         |
| `duplicate_peer_up`                | WARN     | Peer Up for a peer that is already active           |
| `peer_down_without_peer_up`        | WARN     | Peer Down for a peer that was not active            |
| `timestamp_regression`             | WARN     | Timestamp went backwards for a given peer           |

## Terminology

| Term | Meaning |
|------|---------|
| `.obmp` | BMPDoctor's local capture container format. `BMPDOPENBMP1\n` magic + `u32` BE length-prefixed records. |
| `OBMP` | OpenBMP upstream wrapper inside each RouteViews Kafka `*.bmp_raw` payload. Stripped automatically before BMP frame parsing. |
| Inner frame | The RFC 7854 BMP message (common header + per-peer header + body). This is what BMPDoctor's parser operates on. |

## Limitations

BMPDoctor evaluates observed ordering within the input file. If a file starts
mid-session, warnings like `route_monitoring_before_peer_up` may indicate an
incomplete capture rather than a broken BMP feed. Warnings from live RouteViews
captures are expected — they reflect real-world stream ordering, not parser
failures.

Frame-level validation checks the BMP common header (version, length, type) but
does not perform deep BGP attribute validation beyond what BGPKIT Parser provides.

Currently only raw BMP frame files are supported. OpenBMP wrapper files, `.bmpr`
capture format, compressed files (`.bz2`/`.gz`), Kafka, TCP listeners, and
streaming inputs are out of scope for the MVP.

## Sources & capture

- [Data sources reference](docs/sources.md) — RouteViews Kafka (primary), CAIDA (historical)
- [OpenBMP Kafka capture guide](docs/openbmp-kafka-capture.md) — connectivity testing with `nc`/`kcat`
- [Future issues](docs/future-issues.md) — planned features and their scope
- [Real-sample validation](docs/real-sample-validation.md) — verified capture + parse results

## Roadmap (not yet implemented)

### Active next external-data milestones

RouteViews Kafka (`stream.routeviews.org:9092`) is verified reachable.
Broad regex capture with the recorder produced 100 messages, 27,630 bytes.
See [RouteViews Kafka verification](docs/routeviews-kafka-verification.md).

- `record_openbmp_kafka.rs` — **Implemented and verified** (100 msgs, 4s capture)
- `--format openbmp-len` — **Implemented and verified** (100 msgs, 18 peers, 0 malformed)

### Active priority

- Public BMP fixture corpus
- Local FRR/GoBGP `.rawbmp` integration testing
- Compressed input (`.bz2`, `.gz`)
- `.bmpr` capture format support
- TCP listener mode

### Blocked

CAIDA's `bmp.bgpstream.caida.org:9092` is unreachable from the developer's
network (see [verification log](docs/caida-kafka-verification.md)).

- Kafka input (integrated into core CLI)

### Future

- RouteViews `bgpreader` PSV comparison tooling
- Prometheus metrics export
- Parquet export

## License

MIT
