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

## Installation

```sh
cargo install --path .
```

## Usage

### inspect

```sh
bmpdoctor inspect path/to/bmp-data.rawbmp
```

Outputs file metadata, message type counts, per-peer statistics, active peer count,
top peers by route-monitoring messages, and a findings summary.

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
| `OBMP` | OpenBMP upstream wrapper inside each RouteViews Kafka `*.bmp_raw` payload. Stripped by `--format openbmp-len` before BMP frame parsing. |
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

- [Data sources reference](docs/sources.md) — CAIDA/OpenBMP Kafka broker details
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
