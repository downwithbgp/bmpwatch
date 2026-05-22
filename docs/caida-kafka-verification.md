# CAIDA/OpenBMP Kafka Verification

**Result: Unreachable — blocked until a working broker is confirmed.**

## Environment

- **Date:** May 2026
- **Target:** `bmp.bgpstream.caida.org:9092`
- **Resolved IP:** `192.172.226.44`
- **General internet connectivity:** working

## Commands run

### 1. TCP reachability

```sh
$ nc -vz bmp.bgpstream.caida.org 9092
nc: connectx to bmp.bgpstream.caida.org port 9092 (tcp) failed: Network is unreachable
```

### 2. Kafka metadata

```sh
$ kcat -b bmp.bgpstream.caida.org:9092 -L
% ERROR: Failed to acquire metadata: Local: Broker transport failure
```

### 3. DNS resolution

```sh
$ host bmp.bgpstream.caida.org
bmp.bgpstream.caida.org has address 192.172.226.44
```

## Conclusion

- The host `bmp.bgpstream.caida.org` resolves in DNS but is not reachable
  from this network on port 9092.
- This may be due to network-layer filtering, CAIDA-side access policy changes,
  or the broker being decommissioned since the documentation was written.
- The CAIDA/OpenBMP Kafka feed documented in public references (Artemis Private
  BMP feeds, BGPStream V2 docs) appears to be either offline, firewalled, or
  restricted to authorized networks.

## Impact on BMPDoctor

- `examples/record_openbmp_kafka.rs` is **blocked** until a reachable OpenBMP
  Kafka broker is confirmed.
- `--format bmpd` (`.bmpd` file support) was blocked until RouteViews
  Kafka was verified reachable; now implemented and verified against live data.
  it depends on having real OpenBMP-wrapped payloads for testing.
- CAIDA/OpenBMP Kafka is **not** the preferred real-data path for BMPDoctor.
  Local FRR/GoBMP `.rawbmp` captures are the recommended integration test source.
- Public RouteViews real-time access works through `bgpreader -p routeviews-stream`,
  but that produces decoded event records, not raw BMP frames.
