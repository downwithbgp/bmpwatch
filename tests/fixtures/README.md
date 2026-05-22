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

## Manual validation

All fixtures can be inspected offline with `bmpdoctor`. These are
deterministic checks — no network required.

```sh
# OpenBMP-wrapped in .bmpd container
cargo run --bin bmpdoctor -- \
  inspect tests/fixtures/openbmp-two-records.bmpd --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   container_records=2, openbmp_wrapped_payloads=2, metadata present

# Raw BMP in .bmpd container
cargo run --bin bmpdoctor -- \
  inspect tests/fixtures/init-term-tlvs.bmpd --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   Initiation info: sysDescr/bmp-speaker,
#   Termination info: Reason Administratively closed (code 2)

# Raw BMP frames (no container)
cargo run --bin bmpdoctor -- \
  inspect tests/fixtures/init-term-tlvs.rawbmp --summary-json
# Expected: total_messages=2, malformed_messages=0,
#   format auto-detected as raw-bmp,
#   Initiation/Termination TLV output same as .bmpd variant
```
