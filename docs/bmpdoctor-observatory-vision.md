# BMPWatch Observatory — Vision

BMPWatch Observatory is a proposed future public learning and diagnostic
web UI for watching selected real-time BMP/BGP telemetry streams and
understanding what is happening. It does not exist yet.

## Audience

- NOC engineers learning BMP and global routing telemetry
- Network engineering students
- Researchers exploring routing data at scale
- Routing-curious operators
- People who know BGP but have never seen BMP made readable

## Stream / source model

BMPWatch's native area is raw BMP and OpenBMP streams. The Observatory
may also display companion BGP event streams for comparison and teaching,
clearly labeled as such.

| Source class | Examples | Format | Role |
|-------------|----------|--------|------|
| Raw BMP / OpenBMP | RouteViews Kafka `*.bmp_raw` topics | BMPWatch `.bmpd` / `OBMP` unwrap | Primary input |
| Synthetic / curated replay | `.bmpd` files with known scenarios | BMPWatch `.bmpd` | Teaching and demos |
| BGP event streams | BGPStream `routeviews-stream`, RIPE RIS Live | Decoded BGP events, not BMP | Comparison / teaching view |

The distinction between raw BMP and decoded BGP event streams is
critical. The Observatory should not blur them — each view should make
its data source and limitations clear.

## Core UI

Users select a source (collector name, topic, or curated replay):

- Live health line (`OK`, `OK_WITH_STREAM_WARNINGS`, `ISSUES`)
- Messages per second
- Message type mix (Route Monitoring, Peer Up/Down, Initiation,
  Termination, Stats Report)
- Peers observed, active peers
- Peer Up / Peer Down events with reason codes
- Route Monitoring flow (prefixes announced/withdrawn)
- Stats Report counters
- Findings explained in plain English
- Collector / router / OpenBMP metadata where available

## "Explain what I am seeing" layer

Common situations explained in human-readable annotations:

- Mid-stream capture warnings (RM before Peer Up)
- Duplicate Peer Up
- Peer Down reasons
- Timestamp regression
- Malformed messages
- OpenBMP `OBMP` wrapper vs inner BMP frame
- BMP vs MRT / BGPReader / RIS event streams

## MVP architecture

Deliberately small:

- One live ingest process using existing recorder/parser logic
- Rolling in-memory window only (no full archive)
- SSE or WebSocket to browser
- JSON summaries reused from BMPWatch CLI
- Curated list of allowed public sources
- No arbitrary user uploads in MVP

## Privacy and safety

- Public page uses only public/curated streams
- No acceptance or storage of private BMP captures without an
  explicit privacy model
- Not positioned as a monitoring replacement
- Rate-limited and memory-bounded

## Non-goals

- Not an OpenBMP replacement
- Not a RouteViews / BGPStream replacement
- Not a Prometheus / Grafana / Parquet observability platform
- No deep BGP UPDATE semantic validation
- No long-term archival search
- No arbitrary multi-tenant capture hosting

## Near-term path

1. Stabilize CLI JSON outputs (done — 100 tests, clean tree)
2. Add live `watch` / stream summary mode to CLI (future)
3. First web prototype can consume a curated `.bmpd` replay
4. Connect to RouteViews live Kafka
5. Add source selector UI
