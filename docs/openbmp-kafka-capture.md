# OpenBMP Kafka Capture

How to verify reachability and test capture from an OpenBMP Kafka broker
before integrating into BMPDoctor.

## Preferred broker: RouteViews

`stream.routeviews.org:9092` is the verified working broker for
BMPDoctor integration. See
[RouteViews Kafka verification](routeviews-kafka-verification.md)
for the full test log.

**Topic regex:** `^route-?views\..*\.bmp_raw$`

**Warning:** Topic naming is not perfectly uniform. Both `routeviews.`
and `route-views.` prefixes have been observed. Consumers must handle
both variants.

## Historical: CAIDA BGPStream broker

`bmp.bgpstream.caida.org:9092` was tested in May 2026 and found
unreachable from the developer's network. It is preserved in
documentation for historical reference only. See
[CAIDA Kafka verification](caida-kafka-verification.md).

## Prerequisites

Install `kcat` (formerly `kafkacat`):

```sh
# macOS
brew install kcat

# Debian/Ubuntu
sudo apt-get install kafkacat
```

## 1. Broker reachability test

```sh
nc -vz stream.routeviews.org 9092
```

## 2. List available topics

```sh
kcat -b stream.routeviews.org:9092 -L
```

Look for topics matching `^route-?views\..*\.bmp_raw$`.

## 3. Consume a single topic (one peer session)

Pick a topic from the listing and subscribe:

```sh
kcat -b stream.routeviews.org:9092 \
  -t routeviews.sg.64050.bmp_raw \
  -C -o beginning -c 10
```

Flags:
- `-C`: consumer mode
- `-o beginning`: start from earliest available offset
- `-c 10`: consume 10 messages only

Each message printed will be binary BMP frame data. Pipe through `xxd` or
`hexdump -C` for inspection:

```sh
kcat -b stream.routeviews.org:9092 \
  -t routeviews.sg.64050.bmp_raw \
  -C -o beginning -c 1 | xxd | head -20
```

## 4. Consume multiple peer sessions (regex subscribe)

`kcat` does not support topic regex natively, but you can list topics and
spawn one consumer per topic, or use `kafka-console-consumer` with
`--whitelist`:

```sh
kafka-console-consumer \
  --bootstrap-server stream.routeviews.org:9092 \
  --whitelist 'routeviews\..*\.bmp_raw' \
  --from-beginning \
  --max-messages 100
```

## 5. Capture to a local .obmp file

```sh
kcat -b stream.routeviews.org:9092 \
  -t routeviews.sg.64050.bmp_raw \
  -C -o beginning -c 1000 \
  > captured_peer.obmp
```

Then inspect with BMPDoctor (once `--format openbmp-len` is implemented):

```sh
# Future: bmpdoctor inspect captured_peer.obmp --format openbmp-len
# Future: bmpdoctor dump captured_peer.obmp --format openbmp-len --jsonl | head -5
```

## Notes

- Messages arrive as raw BMP frames (common header + payload). No OpenBMP
  length-delimited wrapper is applied to individual Kafka messages.
- When captured to a `.obmp` file, the `examples/record_openbmp_kafka.rs`
  tool (future) will add the `BMPDOPENBMP1` + `u32` BE length wrapper to
  each frame on write.
- The broker may throttle or close connections that consume too fast.
- For production capture, `examples/record_openbmp_kafka.rs` (future) will
  handle reconnection, offset tracking, and rotation.
