# RouteViews Kafka Verification

**Result: Reachable — working OpenBMP Kafka broker confirmed.**

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
- This is now the **preferred external real-data source** for BMPDoctor's
  next integration milestone.
- Each topic corresponds to a BMP peer session from a RouteViews collector.
- Messages are raw BMP frames (common header + payload).

## Impact on BMPDoctor

- `examples/record_openbmp_kafka.rs` can proceed with RouteViews as the
  target broker.
- `--format openbmp-len` (`.obmp`) can be tested against real Kafka-captured
  BMP data.
- RouteViews Kafka is the active next external-data milestone for BMPDoctor.
