# OpenBMP Kafka Capture

**Status: Blocked.** `bmp.bgpstream.caida.org:9092` is not reachable from
the developer's network as of May 2026. See
[CAIDA Kafka verification](caida-kafka-verification.md) for the test log.

The procedures below are preserved for reference. They should be re-run if a
reachable OpenBMP broker is confirmed in the future.

How to verify reachability and test capture from CAIDA's public OpenBMP Kafka
broker before integrating into BMPDoctor.

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
nc -vz bmp.bgpstream.caida.org 9092
```

If the broker is reachable, `nc` will print a success message. If not, the
connection will time out or be refused. Record the actual output.

## 2. List available topics

```sh
kcat -b bmp.bgpstream.caida.org:9092 -L
```

Look for topics matching `openbmp.router--*.peer-as--*.bmp_raw`. Record the
actual topic list.

## 3. Consume a single topic (one peer session)

Pick a topic from the listing and subscribe:

```sh
kcat -b bmp.bgpstream.caida.org:9092 \
  -t openbmp.router--abc123.peer-as--65000.bmp_raw \
  -C -o beginning -c 10
```

Flags:
- `-C`: consumer mode
- `-o beginning`: start from earliest available offset
- `-c 10`: consume 10 messages only

Each message printed will be binary BMP frame data. Pipe through `xxd` or
`hexdump -C` for inspection:

```sh
kcat -b bmp.bgpstream.caida.org:9092 \
  -t openbmp.router--abc123.peer-as--65000.bmp_raw \
  -C -o beginning -c 1 | xxd | head -20
```

## 4. Consume multiple peer sessions (regex subscribe)

`kcat` does not support topic regex natively, but you can list topics and
spawn one consumer per topic, or use `kafka-console-consumer` with
`--whitelist`:

```sh
kafka-console-consumer \
  --bootstrap-server bmp.bgpstream.caida.org:9092 \
  --whitelist 'openbmp\.router--.+\.peer-as--.+\.bmp_raw' \
  --from-beginning \
  --max-messages 100
```

## 5. Capture to a local .obmp file

```sh
kcat -b bmp.bgpstream.caida.org:9092 \
  -t openbmp.router--abc123.peer-as--65000.bmp_raw \
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
