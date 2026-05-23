# Replay / Watch Design

Future CLI concept for replaying `.bmpd` or `.rawbmp` captures as a
rolling window stream — a stepping stone toward live Observatory
summaries.

## Command sketch

```sh
bmpdoctor watch samples/capture.bmpd \
  --window-messages 100 \
  --interval 1s
```

- Reads an existing capture file sequentially.
- Maintains a bounded rolling window of the last N messages.
- Emits periodic summaries (human or JSON lines).

## Why replay before live

1. Lets us develop live-style summaries against deterministic
   fixtures and committed captures — no network dependency.
2. Supports Observatory later without requiring live source
   integration first.
3. Avoids building a long-running streaming collector too early.
4. The existing parser, state, and findings logic can be reused
   directly.

## Rolling window state

Per window (last N messages or time interval):

- Message type counts (`by_type`)
- Peers observed / active within window
- Findings buckets (`parse_errors`, `stream_order_warnings`,
  `other_findings`) in window
- Metadata (collector, router) if present
- Messages per second (if source timing exists; otherwise replay
  rate only)

The rolling window is separate from whole-file inspector state.
It resets or slides as messages leave the window.

## Proposed JSON output

Minimal, additive to existing `--summary-json` shape:

```json
{
  "window_messages": 100,
  "total_seen": 500,
  "elapsed_ms": 4500,
  "by_type": {"RouteMonitoring": 87, "PeerUp": 2},
  "peers_observed": 18,
  "findings_buckets": {
    "parse_errors": 0,
    "stream_order_warnings": 5,
    "other_findings": 0
  },
  "metadata": {
    "collector": "bmp-01",
    "router": "namex.fco"
  }
}
```

## Limitations

- Replaying a file is not the same as live BMP capture — arrival
  timing is artificial unless timestamps are embedded in the source.
- `.bmpd` records do not currently preserve original arrival
  timestamps; replay timing is driven by the replay rate, not Kafka
  or network latency.
- Messages per second should not be interpreted as real-world
  throughput unless source timing metadata exists.
- Rolling-window findings may differ from whole-file findings
  (e.g., a Peer Down in window 1 may look like a missing Peer Up
  until window 2 includes it).

## Implementation notes

- Rolling model implemented in `src/rolling.rs` (`RollingSummary`).
- CLI command still future work.
- Reuse existing parser and state tracking logic where possible.
- Do not duplicate rule matching code.
- Keep memory bounded to `--window-messages` records.
- Start with file replay before live Kafka integration.
- Do not introduce web server or SSE/WebSocket concepts yet.
- Emit JSON lines to stdout for easy piping to external consumers.

## Testing plan

- Deterministic tests with committed fixtures and small synthetic
  captures.
- Rolling window count verification (N messages in, N out).
- Window boundary behavior (message exactly at window edge).
- Malformed entries within a valid stream (should not hang).
- Max window size and default behavior.
- `.rawbmp` and `.bmpd` input parity.
