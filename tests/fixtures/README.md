# Committed regression fixtures

These `.bmpd` files are synthetic, deterministic test assets. They are
**read-only** during `cargo test`. Generated or captured output belongs in
`samples/` or a temporary directory, not here.

No fixture contains live or third-party capture data. All peer ASNs are
private/synthetic.

## `openbmp-two-records.bmpd`

| Property | Value |
|----------|-------|
| Container | `.bmpd` |
| Records | 2 |
| Payload type | OpenBMP `OBMP`-wrapped |
| Record 1 | Peer Up Notification, AS65000 |
| Record 2 | Route Monitoring, AS65000 |
| Size | 351 bytes |

**Purpose:** Regression test for `.bmpd` container parsing, upstream `OBMP`
header stripping, and inner RFC 7854 BMP frame extraction through
bgpkit-parser's `parse_openbmp_header`.

**Validated by:** `obmp_reader::tests::test_committed_fixture_two_openbmp_records`

## `init-term-tlvs.bmpd`

| Property | Value |
|----------|-------|
| Container | `.bmpd` |
| Records | 2 |
| Payload type | Raw BMP (no `OBMP` wrapper) |
| Record 1 | Initiation Message: sysDescr `FRRouting`, sysName `bmp-speaker` |
| Record 2 | Termination Message: reason code 2 (Administratively closed) |
| Size | 67 bytes |

**Purpose:** Regression test for base RFC 7854 Initiation and Termination
TLV decoding (sysDescr, sysName, termination reason names).

**Validated by:** `doctor::tests::test_init_term_tlv_fixture`

## `init-term-tlvs.rawbmp`

| Property | Value |
|----------|-------|
| Container | None — concatenated raw RFC 7854 BMP frames |
| Records | 2 |
| Payload type | Raw BMP |
| Record 1 | Initiation Message: sysDescr `FRRouting`, sysName `bmp-speaker` |
| Record 2 | Termination Message: reason code 2 (Administratively closed) |
| Size | 46 bytes |

**Purpose:** Regression test for raw BMP input (`--format raw-bmp`) with
auto-detection and RFC 7854 TLV decoding. Mirrors `init-term-tlvs.bmpd`
without the `.bmpd` container framing.

**Validated by:** `doctor::tests::test_init_term_tlv_rawbmp_fixture`

## `peer-up-down.rawbmp`

| Property | Value |
|----------|-------|
| Container | None — concatenated raw RFC 7854 BMP frames |
| Records | 2 |
| Payload type | Raw BMP |
| Record 1 | Peer Up Notification, AS65000 |
| Record 2 | Peer Down Notification, AS65000, reason code 2 (Local system closed, no NOTIFICATION) |
| Size | 127 bytes |

**Purpose:** Regression test for clean peer lifecycle (Peer Up → Peer Down)
with no stream-order warnings. Verifies peer state tracking: active=false
at end, no `peer_down_without_peer_up` finding.

This fixture validates BMP Peer Up/Down lifecycle behavior. It may produce
a bgpkit-parser warning for the synthetic embedded BGP OPEN message; that
is not a BMP framing or lifecycle failure.

**Validated by:** `doctor::tests::test_peer_up_down_rawbmp_fixture`

## `stats-report.rawbmp`

| Property | Value |
|----------|-------|
| Container | None — raw RFC 7854 BMP frames |
| Records | 1 |
| Payload type | Raw BMP |
| Record 1 | Stats Report: Adj-RIBs-In = 42, Loc-RIB = 10 |
| Size | 68 bytes |

**Purpose:** Regression test for RFC 7854 Stats Report decoding.
No Peer Up message included — avoids synthetic BGP OPEN parse warnings.

**Validated by:** `doctor::tests::test_stats_report_rawbmp_fixture`

## Malformed fixtures

These are intentionally invalid samples that validate BMPWatch's error
handling. All produce findings or errors; none should panic.

| File | Format | Fault | Expected behavior |
|------|--------|-------|-------------------|
| `malformed/truncated-common-header.rawbmp` | raw BMP | 4 bytes (< 6) | `truncated_frame` finding, exit 2 |
| `malformed/bad-version.rawbmp` | raw BMP | Version 0xFF | `invalid_bmp_version` finding |
| `malformed/truncated-per-peer-header.rawbmp` | raw BMP | Payload < 42 bytes | Frame parsed, no per-peer header, no peer tracked |
| `malformed/bad-magic.bmpd` | `.bmpd` | Wrong container magic | `ObmpReader::open` returns error. Auto-detection falls back to raw BMP; use `--format bmpd` to test container parsing. |
| `malformed/truncated-record-length.bmpd` | `.bmpd` | Partial length prefix | Iterator yields error frame |

```sh
cargo run --bin bmpwatch -- \
  lint tests/fixtures/malformed/bad-version.rawbmp
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/malformed/truncated-common-header.rawbmp --summary-json
```

```sh
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/stats-report.rawbmp --summary-json
# Expected: total_messages=1, malformed_messages=0,
#   Stats Report info: Adj-RIBs-In=42, Loc-RIB=10

```sh
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/peer-up-down.rawbmp --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   peers_observed=1, active_peers=0, stream_order_warnings=0,
#   session_lifecycle.peer_up_messages=1, peer_down_messages=1,
#   Peer Down info: Local system closed, no NOTIFICATION (code 2)
# (may include 1 bgpkit-parser parse_error from synthetic BGP OPEN)
```

## Manual validation

All fixtures can be inspected offline with `bmpwatch`. These are
deterministic checks — no network required.

```sh
# OpenBMP-wrapped in .bmpd container
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/openbmp-two-records.bmpd --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   container_records=2, openbmp_wrapped_payloads=2, metadata present

# Raw BMP in .bmpd container
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/init-term-tlvs.bmpd --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   Initiation info: sysDescr/bmp-speaker,
#   Termination info: Reason Administratively closed (code 2)

# Raw BMP frames (no container)
cargo run --bin bmpwatch -- \
  inspect tests/fixtures/init-term-tlvs.rawbmp --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   format auto-detected as raw-bmp,
#   Initiation/Termination TLV output same as .bmpd variant
```
