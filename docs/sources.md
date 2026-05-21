# BMP Data Sources

This document catalogs known sources of raw BMP data that BMPDoctor targets.
These are not built into the core CLI; they are experimental references for
capture, relay, or integration work.

## CAIDA / BGPStream OpenBMP Kafka

CAIDA operates a public BMP feed as part of Artemis Private BMP feeds.

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

- The broker at `bmp.bgpstream.caida.org:9092` is publicly reachable.
- Topics are produced in real-time; no historical offset retention guarantee.
- Authentication is not required for read-only consumers.
- Use `kcat` (librdkafka) or `kafka-console-consumer` for testing.

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

## Raw BMP frame files

Concatenated BMP frames with no wrapper:
```
[Common Header (6)] [Payload] [Common Header (6)] [Payload] ...
```

This is the primary format BMPDoctor targets in the MVP.
