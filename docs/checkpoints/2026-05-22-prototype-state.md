# Prototype State — 2026-05-22

**Commit:** `97d981b`  
**Tests:** 99/99, clean tree

## What is solid

- `.rawbmp` auto-detect and RFC 7854 frame parsing
- `.bmpd` container read/write (`BMPDOPENBMP1\n` + u32 BE length prefix)
- OpenBMP `OBMP` unwrap for RouteViews Kafka payloads (bgpkit-parser header strip)
- RouteViews Kafka recorder with topic discovery, filters, safety guards
- `inspect` / `lint` / `dump --jsonl` working
- Health line, session lifecycle, peer inventory, findings buckets
- Initiation / Termination TLV decoding (sysDescr, sysName, reason codes)
- Stats Report basic decoding (13 known stat types)
- 5 valid committed fixtures, 2 `.bmpd` + 3 `.rawbmp`
- 5 malformed committed fixtures
- RawBmpIterator EOF/truncation regression fixed

## Validation sources

| Source | Status |
|--------|--------|
| RouteViews Kafka `.bmpd` | Verified: 100 msgs, 18 peers, 0 malformed, metadata present |
| Synthetic `.rawbmp` fixtures | 3 committed: init-term-tlvs, peer-up-down, stats-report |
| Synthetic `.bmpd` fixtures | 2 committed: OBMP-wrapped PeerUp+RM, init-term-tlvs |
| Malformed fixtures | 5 committed: truncated, bad version, bad magic |
| FRR local lab | BGP verified; BMP output not verified |

## Known caveats

- No deep BGP UPDATE semantic validation
- No native PCAP/TCP reassembly
- `.bmpd` is BMPWatch local container, not an OpenBMP standard
- FRR BMP output blocked on tested images (8.4_git, 10.6.1_git)
- Synthetic Peer Up fixtures may have bgpkit-parser parse_error from minimal embedded BGP OPEN

## Recommended next tasks (ranked)

1. Surrender Peer Down reason names in peer inventory / lifecycle display
2. More RouteViews multi-collector validation notes
3. Malformed `.bmpd` summary/report polish
4. Optional: standalone TCP BMP collector (only if a testable BMP speaker becomes available)
5. Defer: PCAP native support, Prometheus/Parquet, extension RFCs
