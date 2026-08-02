# Changelog

## v0.1.2

Security hardening, dependency slimming, and robustness fixes.

### Security

- Cap declared BMP frame length at 64 MiB before allocation (`RawBmpIterator`), rejecting crafted capture files that previously forced multi-GiB allocations.
- Cap RTR (RFC 8210) PDU length at 64 KiB and VRP count at 2M; a malicious RTR server can no longer force huge allocations or unbounded memory growth.
- Bound RPKI cache loading: `Vec::with_capacity` is limited to what the file can actually hold, so a tampered cache cannot reserve multi-GiB.
- Write the RPKI cache atomically (temp file + rename) with `0600` permissions; the temp write refuses to follow symlinks, self-heals stale temps, and fsyncs before the rename.
- Sanitize control characters (ANSI/OSC escape injection) in TLV strings and OpenBMP metadata at every terminal output sink.
- Make AS-name truncation UTF-8-safe in the dashboard (no more panic on multi-byte names).
- Bound live-stream state: peer/prefix/AS tracking maps are capped (100k peers/ASNs, 1M prefixes), stored AS paths are truncated at 64 ASNs, and the live Kafka payload path enforces the 64 MiB cap.
- Cap `dump --jsonl` event buffering at 100k events (new `--max-events` flag) so large captures cannot OOM the process.
- Clamp `--window-messages` to 1M; oversized values are rejected at parse time.
- Exit cleanly (code 0) on broken pipe instead of panicking with a backtrace.
- Harden CI: all third-party actions SHA-pinned, cargo-dist and rustup installers checksum-verified, least-privilege `GITHUB_TOKEN` on CI.
- Fix an unreachable-in-practice advisory: bump `anyhow` to 1.0.104 (RUSTSEC-2026-0190).

### Dependencies

- Slim `bgpkit-parser` to its `parser` feature, dropping 126 transitive crates (oneio, reqwest, hyper, h2, tokio, tower, aws-lc-rs, and more) from the build.
- Drop ureq's unused gzip feature and rdkafka's unused tokio feature (sync consumer only).
- Add `deny.toml` license and advisory policy; `cargo deny check` passes all four checks.

### Fixes

- Restore per-finding peer warning counts in the dashboard (regression from the state-bounding change).

## v0.1.1

Parser hardening and doctor diagnostics hotfix.

- Prevent malformed Statistics Report messages from panicking the parser.
- Reject oversized `.bmpd` payload lengths before allocation.
- Avoid false-positive doctor parse errors by only validating route-bearing BMP messages with bgpkit-parser.
- Report truncated Peer Down Notifications that omit the mandatory reason byte.
- Add regression coverage for malformed BMP/OpenBMP inputs.
