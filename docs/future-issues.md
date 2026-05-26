# Future Issues

Historical tags (`v0.1.*`) are checkpoints, not polished releases.

## Implemented (archived)

### 1. record_openbmp_kafka.rs
**Status:** Implemented and verified. Standalone binary at `src/bin/record_openbmp_kafka.rs`.

### 2. --format bmpd
**Status:** Implemented and verified. Container parsing + OpenBMP unwrap.

---

## A. Near-term (good next BMPWatch work)

### Better RFC 7854 message summaries
- More complete TLV display for Initiation/Termination
- Peer Down reason code names in inspect output
- Stats Report counter decoding

### More synthetic fixtures
- Additional committed `.bmpd` regression fixtures
- Edge cases: mid-stream start, timestamp regression, IPv6 peers

### Peer/session lifecycle summaries
- Session duration per peer
- Peer state transition timeline in human output

### Real RouteViews validation notes
- Document additional sample captures with different collectors/ASNs
- Validate Peer Up / Peer Down message counts in longer captures

---

## B. Useful later

### Compressed input support (`.bz2`, `.gz`)
**Status:** Not started. Use `bzip2` / `flate2` crates for transparent decompression.

### `.bmpr` capture format support
**Status:** Not started.

### PCAP / PCAPNG support
**Status:** Deferred. See [docs/pcap-support.md](pcap-support.md).

### Standalone `OBMP` payload file support
**Status:** Not started. Depends on having real standalone OBMP payload files.

### RouteViews `bgpreader` PSV comparison tooling
**Status:** Not started.

### More output formats
**Status:** Not started. Only if there is a clear consumer.

### Public BMP fixture corpus
**Status:** Not started. Collect/generate fixtures for edge cases.

---

## C. Explicitly not now

- Deep BGP UPDATE semantic validation
- Full observability platform / storage backend
- Prometheus metrics / Parquet export
- TCP listener mode
- RFC 8671 / 9069 / 9736 interpretation until base RFC 7854 behavior is mature
- CAIDA Kafka integration (`bmp.bgpstream.caida.org:9092` remains unreachable)
- FRR BMP local lab: BGP verified; BMP output blocked on tested FRR images
  (8.4_git, 10.6.1_git) — see `docs/frr-local-bmp-lab.md`
