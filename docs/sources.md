# BMP Data Sources

This document catalogs known sources of raw BMP data that BMPDoctor targets.
These are not built into the core CLI; they are experimental references for
capture, relay, or integration work.

## Source ladder

Sources ordered by implementation status and verification level.

| Tier | Source                             | Extension    | Status                  |
|------|------------------------------------|--------------|-------------------------|
| 1    | Synthetic fixtures                 | (in-memory)  | Implemented, 17 tests   |
| 2    | Local FRR/GoBGP raw BMP            | `.rawbmp`    | Planned, not tested     |
| 3    | CAIDA/OpenBMP Kafka                | `.obmp`      | Unverified, see below   |
| 4    | BGPReader routeviews-stream        | N/A          | Comparison only         |

### Tier 1: Synthetic fixtures

Generated in test code (`raw_bmp.rs` fixtures module). Used for frame
validation, peer state tracking, and lint rule coverage. No network
dependency.

### Tier 2: Local FRR/GoBGP `.rawbmp`

BMP speakers like FRR and GoBGP can write raw BMP frame files to disk. These
provide real BGP data for integration testing. The `.rawbmp` extension
distinguishes raw concatenated BMP frames from OpenBMP length-delimited
captures.

### Tier 3: CAIDA/OpenBMP Kafka `.obmp`

See [OpenBMP Kafka capture guide](openbmp-kafka-capture.md). `.obmp` means
length-delimited OpenBMP Kafka payloads captured to local disk. This source
has not been verified from this network yet.

### Tier 4: BGPReader routeviews-stream / RouteViews (comparison only)

RouteViews publishes MRT RIB dumps (every 2 hours) and MRT update dumps
(every 15 minutes). These are useful for BGPKIT Parser testing but are
**not raw BMP input**. Real-time BMP data from RouteViews collectors is
accessed via BGPStream, not direct collector connections.

`bgpreader -p routeviews-stream` produces decoded BGP event streams. Useful
for comparing BMPDoctor output against known-good BGP data, but NOT a raw
BMP input source. No integration planned.

See [RouteViews and BMPDoctor](routeviews.md) for details on archive data,
real-time data, and BMPDoctor implications.

---

## CAIDA / BGPStream OpenBMP Kafka

**Verification pending.** The information below is based on public
documentation. Connectivity has not been confirmed from this network.

- **Host:** `bmp.bgpstream.caida.org`
- **Port:** `9092`
- **Topic regex:** `^openbmp\.router--.+\.peer-as--.+\.bmp_raw`

### Kafka topic structure

Each Kafka topic corresponds to a single BMP peer session. The naming convention is:

```
openbmp.router--<router_hash>.peer-as--<peer_asn>.bmp_raw
```

Messages in each partition are raw BMP frames (common header + payload)
streamed by OpenBMP, typically length-delimited by the Kafka message framing.

### Access notes

- The broker at `bmp.bgpstream.caida.org:9092` is documented as publicly
  reachable. This has not been verified from this network.
- Topics are produced in real-time; no historical offset retention guarantee.
- Authentication is reportedly not required for read-only consumers.
- Use `kcat` (librdkafka) or `kafka-console-consumer` for testing.

### Manual verification required

Before any integration work, the following must be run manually and the
output recorded:

```sh
nc -vz bmp.bgpstream.caida.org 9092
kcat -b bmp.bgpstream.caida.org:9092 -L
```

See `docs/openbmp-kafka-capture.md` for full connectivity test procedures.

### References

- [Artemis Private BMP feeds](https://bgpstream.caida.org/docs/api/artemis-private-bmp-feeds)
- [BGPStream V2 docs - realtime OpenBMP Kafka](https://bgpstream.caida.org/docs/api/bgpstreamv2)
- [OpenBMP Kafka message format](https://www.openbmp.org/#!docs/message_bus.md)

## OpenBMP length-delimited files (.obmp)

OpenBMP can write BMP data to disk with a length-delimited wrapper:

```
Magic:      "BMPDOPENBMP1"  (12 bytes)
Length:     u32 big-endian   (4 bytes)
Payload:    raw BMP frame    (Length bytes)
```

Each record is a single BMP frame prefixed by the magic header and length.
BMPDoctor does not yet support this format; see `docs/future-issues.md`.

## Raw BMP frame files (.rawbmp)

Concatenated BMP frames with no wrapper:
```
[Common Header (6)] [Payload] [Common Header (6)] [Payload] ...
```

This is the primary format BMPDoctor targets in the MVP. Files use the
`.rawbmp` extension by convention when captured from a local BMP speaker.
