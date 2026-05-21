# OpenBMP Kafka Capture

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
nc -zv bmp.bgpstream.caida.org 9092
```

Expected output:
```
Connection to bmp.bgpstream.caida.org port 9092 [tcp/XmlIpcRegSvc] succeeded!
```

## 2. List available topics

```sh
kcat -b bmp.bgpstream.caida.org:9092 -L
```

Look for topics matching `openbmp.router--*.peer-as--*.bmp_raw`.

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

## 5. Capture to a local file

```sh
kcat -b bmp.bgpstream.caida.org:9092 \
  -t openbmp.router--abc123.peer-as--65000.bmp_raw \
  -C -o beginning -c 1000 \
  > captured_peer.bin
```

Then inspect with BMPDoctor:

```sh
bmpdoctor inspect captured_peer.bin
bmpdoctor dump captured_peer.bin --jsonl | head -5
```

## Notes

- Messages arrive as raw BMP frames (common header + payload). No OpenBMP
  length-delimited wrapper is applied to individual Kafka messages.
- The broker may throttle or close connections that consume too fast.
- For production capture, `examples/record_openbmp_kafka.rs` (future) will
  handle reconnection, offset tracking, and rotation.
