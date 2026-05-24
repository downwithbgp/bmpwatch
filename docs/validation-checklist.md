# Validation Checklist

Procedures for verifying BMPWatch correctness against known references.
Each section should be completed before claiming production readiness for
that data source.

## 1. Synthetic frame fixtures

- [x] 6-byte common header parsed correctly (version, length, type)
- [x] 42-byte per-peer header fields extracted (peer AS, IP, timestamp)
- [x] Message type classification 0–6
- [x] IPv4 address extraction from IPv4-mapped IPv6 format
- [x] Frame with invalid version → `invalid_bmp_version` finding
- [x] Frame with declared length < 6 → `truncated_frame` finding
- [x] Frame with declared length exceeding file → `truncated_frame` finding
- [x] Unknown message type → `unknown_bmp_type` finding
- [x] Peer lifecycle: Peer Up → Route Monitoring → Peer Down
- [x] Peer lifecycle: Route Monitoring before Peer Up → warning
- [x] Peer lifecycle: Duplicate Peer Up → warning
- [x] Peer lifecycle: Peer Down without Peer Up → warning
- [x] Timestamp regression within same peer → warning
- [x] Initiation message (type 4, no per-peer header)
- [x] Multiple peers in single file
- [x] JSONL dump produces valid JSON per message

### Multi-frame edge cases (added in hardening pass)

- [x] Two valid frames concatenated, same peer
- [x] Valid frame followed by truncated frame
- [x] Malformed frame followed by extra trailing bytes
- [x] Route monitoring before observed Peer Up (synthetic)

## 2. Raw BMP capture from local FRR/GoBGP

- [ ] Configure FRR or GoBGP to write BMP to a file
- [ ] Establish at least one BGP session
- [ ] Capture `.rawbmp` output for 60+ seconds
- [ ] Run `bmpwatch inspect capture.rawbmp`
  - Verify peer count matches expected sessions
  - Verify Peer Up / Peer Down counts are consistent
  - Verify no unexpected lint warnings
- [ ] Run `bmpwatch dump capture.rawbmp --jsonl`
  - Verify timestamps are monotonically increasing per peer
  - Verify parse_status is "ok" for all route monitoring messages
- [ ] Run `bmpwatch lint capture.rawbmp`
  - Verify exit code 0 for a clean session

**Commands to set up FRR BMP output** (reference, not automated):

```
router bgp <ASN>
  bgp router-id <RID>
  neighbor <PEER_IP> bmp
  bmp mirror buffer-limit 0
  bmp targets bmpwatchtest
  bmp listener 127.0.0.1 port 5000
```

Then capture with `nc` or `socat`:

```sh
nc -l 5000 > capture.rawbmp
```

## 3. RouteViews Kafka verification

- [x] Run `nc -vz stream.routeviews.org 9092` — **Connection succeeded**
- [x] Run `kcat -b stream.routeviews.org:9092 -L` — **Topics listed**
- [x] Observed topics match `^route-?views\..*\.bmp_raw$`
- [x] Broad regex capture with recorder — **100 msgs, 27,630 bytes, 4s**
- [x] `.bmpd` container parsing via `--format bmpd` — **100 msgs, 18 peers, 0 malformed**
- [ ] Subscribe to exact topic and capture 100 messages
- [ ] Verify messages are valid BMP frames (inspect first 5 with `xxd`)
- [ ] Run `bmpwatch inspect` on captured `.bmpd` (format auto-detected)
- [ ] Confirm peer addresses and ASNs match expected RouteViews feed data

See `docs/routeviews-kafka-verification.md` for the full test log.

### CAIDA/OpenBMP Kafka — BLOCKED (historical)

- [x] `nc -vz bmp.bgpstream.caida.org 9092` — **Network unreachable**
- [x] `kcat -b bmp.bgpstream.caida.org:9092 -L` — **Broker transport failure**

See `docs/caida-kafka-verification.md`.

## 4. BGPReader routeviews-stream comparison

- [ ] Run `bgpreader -p routeviews-stream -o upd-file=bgp_updates.txt`
- [ ] Capture BMP from a route-views-like BGP session simultaneously
- [ ] Compare announced/withdrawn prefix counts between BGPReader output
      and BMPWatch BGP element counts
- [ ] Verify BGPReader timestamps are within tolerance of BMP timestamps

## 5. RFC 7854 conformance checks

- [ ] Version field: only 3 accepted; any other value produces error
- [ ] Message length: rejected if < 6; truncation detected if exceeds file
- [ ] Per-peer header: present for types 0,1,2,3,6; absent for 4,5
- [ ] Peer address: IPv4 correctly extracted from `::ffff:x.x.x.x` format
- [ ] Peer address: IPv6 correctly formatted when V flag is set
- [ ] Peer Up: at most one active per peer; duplicate generates warning
- [ ] Peer Down: must follow Peer Up; missing Peer Up generates warning
- [ ] Timestamp: regression within same peer generates warning
- [ ] Unknown message types: reported as warning, do not crash parsing
- [ ] Initiation (type 4): parsed without per-peer header extraction
- [ ] Termination (type 5): parsed without per-peer header extraction

## 6. Known limitations

- [ ] Adj-RIB-Out vs Loc-RIB not distinguished (RFC 8671, RFC 9069)
- [ ] Peer Up TLV parsing not implemented (RFC 9736)
- [ ] Post-policy (L) flag not interpreted beyond noting its presence
- [ ] Stats Report counters not decoded
- [ ] Initiation/Termination TLVs not parsed
- [ ] Route Mirroring TLVs not parsed
- [ ] Peer Down reason codes reported as raw integers, not labels
- [ ] No compression support (`.bz2`, `.gz`)
- [ ] No BMPWatch container format support (`.bmpd`)
- [ ] Findings are capped at `--max-findings` (default 1000); findings
      beyond the cap are silently dropped with a truncation warning
- [ ] No TCP listener or streaming input mode
