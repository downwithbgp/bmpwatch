# Architecture

Project architecture and development guide for BMPWatch.

## Build & test commands

```sh
# Build
cargo build

# Run all tests
cargo test

# Run a single test (any substring match)
cargo test obmp_reader::tests::test_committed_fixture_two_openbmp_records

# Run the main binary
cargo run --bin bmpwatch -- inspect <file> [--summary-json] [--format auto|raw-bmp|bmpd]

# Run the Kafka recorder binary
cargo run --bin record_openbmp_kafka -- --out samples/capture.bmpd --max-messages 100

# Install binaries to ~/.cargo/bin/
cargo install --path .
```

## Architecture

BMP stream viewer and diagnostic tool for BGP Monitoring Protocol (RFC 7854). Two binaries from one crate: `bmpwatch` (main CLI) and `record_openbmp_kafka` (Kafka recorder for public BMP feeds).

### Input pipeline

```
File → format detection (input.rs) → Iterator (RawBmpIterator or ObmpReader)
     → RawBmpFrame → Doctor::process_frame() → report module
```

**Format auto-detection** (`input.rs`): reads first bytes. `BMPDOPENBMP1\n` → `.bmpd` container; `0x03` → raw BMP. The `InputFormat::Auto` variant is resolved before `Doctor` construction — `Doctor` never sees `Auto`.

**Two iterators unified via `FrameSource` enum** (private to `doctor.rs`):
- `RawBmpIterator` (`raw_bmp.rs`) — sequential read of RFC 7854 common header (6 bytes) + payload, file-backed. Yields `RawBmpFrame` structs.
- `ObmpReader` (`obmp_reader.rs`) — reads `.bmpd` container: validates magic, reads u32 BE length-prefixed records. Each record payload is classified as raw BMP (starts `0x03`), OpenBMP `OBMP`-wrapped (starts `OBMP` magic), or unrecognized. OBMP-wrapped payloads are unwrapped via `bgpkit_parser::parse_openbmp_header()` before inner BMP frame parsing. Maintains `ContainerStats` for `--summary-json` output.

### Layer model

`.bmpd` = BMPWatch container (`BMPDOPENBMP1\n` magic + u32 BE length-prefixed records). OBMP = upstream OpenBMP wrapper (RouteViews Kafka payloads). Inner frame = RFC 7854 BMP message. The pipeline strips wrappers transparently; `Doctor` always receives `RawBmpFrame`.

### Core types

- **`RawBmpFrame`** (`raw_bmp.rs:166`) — parsed BMP message: version, msg_len, msg_type (enum or raw u8), optional PerPeerHeader, payload bytes, full_data for bgpkit-parser, and optional TLV/stats/peer-down info.
- **`Doctor`** (`doctor.rs:16`) — owns `DoctorState` and `Vec<JsonlEvent>`. `process()` iterates frames, calls `process_frame()` for each: validates version/type, runs bgpkit-parser, updates per-peer state, generates findings. `collect_events=true` populates JSONL events for `dump --jsonl`.
- **`DoctorState`** (`state.rs:81`) — accumulated counters: total/malformed messages, by_type map, peer states (BTreeMap keyed by PeerKey), findings vec, container stats, TLV info.
- **`PeerKey`** (`state.rs:8`) — (peer_asn, peer_ip, peer_distinguisher) tuple used as map key for per-peer state tracking.
- **`Finding`** (`state.rs:57`) — severity (Info/Warn/Error), rule name (stable snake_case identifier), byte offset, peer context, human message.
- **`JsonlEvent`** (`event.rs:7`) — per-message JSONL output for `dump --jsonl`.

### Module map

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point, delegates to `cli::run()` |
| `cli.rs` | Clap CLI definition (inspect/lint/dump subcommands), format resolution, exit codes |
| `doctor.rs` | Central orchestrator: frame iteration, finding generation, peer lifecycle tracking, JSONL event collection |
| `raw_bmp.rs` | RFC 7854 frame parsing, PerPeerHeader, BmpMessageType enum, RawBmpIterator, TLV/Stats parsing, synthetic fixtures |
| `obmp_reader.rs` | `.bmpd` container reader, OBMP unwrap via bgpkit-parser, ContainerStats, container fixtures |
| `obmp_writer.rs` | `.bmpd` container writer (used by `record_openbmp_kafka`). Magic constant `BMPDOPENBMP1\n`, u32 BE length prefix |
| `state.rs` | Data types: DoctorState, PeerKey, PeerState, Finding, Severity |
| `error.rs` | DoctorError enum (Io / Frame string) |
| `event.rs` | JsonlEvent struct, `emit_jsonl()`, `max_exit_code()` |
| `lint.rs` | Finding factory functions, rule name constants (8 rules) |
| `report.rs` | Text and JSON output for inspect/lint commands, findings bucketing, session lifecycle |
| `rolling.rs` | RollingSummary for bounded-window message aggregation (used by `bmpwatch <file>` replay mode) |
| `input.rs` | Format detection from file content, `file_size_and_format()` |
| `kafka.rs` | Shared Kafka utilities: `fetch_topics`, `apply_filters`, `create_consumer` |
| `browser.rs` | Stream browser TUI (collector pane + stream pane) with search, mouse, and keyboard navigation |
| `rpki.rs` | RTR client (RFC 8210), VRP cache, multi-covering-VRP RPKI prefix validation |
| `dashboard.rs` | Live TUI dashboard: message log, RPKI, Prefix Flaps, AS name resolution (cache-based, no network I/O) |
| `peering.rs` | Active peering filter: fetches RouteViews peering-status page, caches for 15 min, filters Kafka topics |
| `asnames.rs` | Team Cymru bulk WHOIS client for offline AS name cache refresh (`bmpwatch refresh-asnames`) |
| `bin/record_openbmp_kafka.rs` | Separate binary: Kafka consumer for public BMP feeds, writes `.bmpd` via ObmpWriter |

### Lint rules (8)

`invalid_bmp_version` (ERR), `truncated_frame` (ERR), `unknown_bmp_type` (WARN), `parse_error` (ERR), `route_monitoring_before_peer_up` (WARN), `duplicate_peer_up` (WARN), `peer_down_without_peer_up` (WARN), `timestamp_regression` (WARN). Rule names are stable snake_case identifiers used in machine output.

### Findings bucketing

- **parse_errors**: invalid_bmp_version, truncated_frame, unknown_bmp_type, parse_error
- **stream_order_warnings**: route_monitoring_before_peer_up, duplicate_peer_up, peer_down_without_peer_up, timestamp_regression
- **other_findings**: anything else

### Testing patterns

- **Synthetic frame fixtures** — `raw_bmp::fixtures` module constructs valid/invalid BMP frames in memory. Used by unit tests in `doctor.rs` and `raw_bmp.rs` via tempfile.
- **Container fixtures** — `obmp_reader::fixtures` wraps frames in `.bmpd` container format, including `make_openbmp_wrapped()` for OBMP wrapper testing.
- **Committed fixture files** — `tests/fixtures/` contains deterministic `.bmpd` and `.rawbmp` files. Read-only; tests open them by path (e.g., `Path::new("tests/fixtures/openbmp-two-records.bmpd")`).
- Tests use `tempfile` for write-then-read patterns. No mocking — all I/O is real file I/O.
- The bgpkit-parser dependency (`0.16`) is used for: BGP UPDATE element counting (RM messages), OpenBMP header parsing (`parse_openbmp_header`). Synthetic BGP data may produce parse errors from bgpkit — these are expected and not considered BMP framing failures.

### Key dependencies

- `bgpkit-parser` 0.16 — BGP message parsing, OBMP header unwrapping
- `rdkafka` 0.36 — Kafka consumer (record_openbmp_kafka binary only)
- `clap` 4 with derive — CLI
- `bytes`, `serde`, `serde_json`, `anyhow`, `regex`
