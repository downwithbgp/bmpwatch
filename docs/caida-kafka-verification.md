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

## Impact on BMPWatch

- CAIDA/OpenBMP Kafka is unreachable and is **not** the preferred real-data
  path for BMPWatch.
- RouteViews Kafka (`stream.routeviews.org:9092`) is the verified live data
  source. The `record_openbmp_kafka` binary and the TUI dashboard both connect
  to it successfully.
- `.bmpd` container support is implemented and verified against live
  RouteViews data.
