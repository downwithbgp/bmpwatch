# RouteViews Kafka Verification

**Result: Reachable — working broker confirmed, capture and parse verified.**

## Environment

- **Date:** May 2026
- **Target:** `stream.routeviews.org:9092`
- **General internet connectivity:** working

## Commands run

### 1. TCP reachability

```sh
$ nc -vz stream.routeviews.org 9092
Connection to stream.routeviews.org port 9092 [tcp/*] succeeded!
```

### 2. Kafka metadata (topic listing)

```sh
$ kcat -b stream.routeviews.org:9092 -L -m 10
Metadata for all topics (from broker -1: stream.routeviews.org:9092/bootstrap):
  ...
  topic "routeviews.route-views2.saopaulo.199524.bmp_raw" ...
  topic "routeviews.flix.395880.bmp_raw" ...
  topic "routeviews.uaeix.31898.bmp_raw" ...
  topic "routeviews.nwax.13335.bmp_raw" ...
  topic "routeviews.sg.64050.bmp_raw" ...
  topic "route-views.linx.8714.bmp_raw" ...
```

### 3. Observed topic names

| Topic | Prefix style |
|-------|-------------|
| `routeviews.route-views2.saopaulo.199524.bmp_raw` | `routeviews.` |
| `routeviews.flix.395880.bmp_raw` | `routeviews.` |
| `routeviews.uaeix.31898.bmp_raw` | `routeviews.` |
| `routeviews.nwax.13335.bmp_raw` | `routeviews.` |
| `routeviews.sg.64050.bmp_raw` | `routeviews.` |
| `route-views.linx.8714.bmp_raw` | `route-views.` |

### 4. Topic regex

```
^route-?views\..*\.bmp_raw$
```

**Warning:** Topic naming is not perfectly uniform. Both `routeviews.` and
`route-views.` prefixes exist. Any Kafka consumer must handle both variants.

## Conclusion

- `stream.routeviews.org:9092` is reachable and serves live BMP data via
  Kafka topics matching `^route-?views\..*\.bmp_raw$`.
- This is now the **preferred external real-data source** for BMPWatch's
  next integration milestone.
- Each topic corresponds to a BMP peer session from a RouteViews collector.
- Messages are raw BMP frames (common header + payload).

## Impact on BMPWatch

- `record_openbmp_kafka.rs` is **implemented and verified** with the broad
  regex `^routeviews.*\.bmp_raw$`. Successful capture: 100 messages,
  27,630 bytes in 4 seconds. Exact single-topic captures may be quiet;
  broad regex is the recommended smoke test.
- `--format bmpd` (`.bmpd`) parser is **implemented and verified**.
  Captured `.bmpd` files parse correctly: 100 Route Monitoring messages,
  18 peers, 29 BGP elements. RouteViews Kafka `*.bmp_raw` payloads are
  OpenBMP `OBMP`-wrapped, not raw RFC 7854. BMPWatch's OpenBMP unwrap
  uses `bgpkit_parser::parse_openbmp_header` to strip the wrapper,
  then passes the inner RFC 7854 BMP frame to our existing parser.
- Our `.bmpd` is a local capture container (`BMPDOPENBMP1\n` magic +
  `u32` BE length prefix). `OBMP` is the upstream OpenBMP wrapper
  inside each RouteViews Kafka payload.

### Verified recorder smoke test

```sh
cargo run --bin record_openbmp_kafka -- \
  --broker stream.routeviews.org:9092 \
  --topic-regex '^routeviews.*\.bmp_raw$' \
  --out samples/routeviews-broad-100.bmpd \
  --max-messages 100 \
  --max-seconds 60 \
  --from-end
```

**Observed result:** messages_written=100, bytes_written=27630,
duration_secs=4, output_path=samples/routeviews-broad-100.bmpd.
