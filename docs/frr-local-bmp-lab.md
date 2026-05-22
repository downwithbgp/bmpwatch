# FRR Local BMP Lab

Planned workflow for generating real `.rawbmp` BMP data using FRR and
Docker Compose in a private-AS, local-only lab. **Not yet manually verified.**

## Quick start

```sh
cd labs/frr-bmp
docker compose up -d
# Wait 30–60 seconds for BGP session establishment
ls -lh ../../samples/frr-smoke.rawbmp
cargo run --bin bmpdoctor -- \
  inspect ../../samples/frr-smoke.rawbmp --summary-json
```

## Topology

Two FRR containers on a private Docker bridge, plus a capture container
running `socat` to record the BMP TCP byte stream.

```
[frr1 AS65000 172.30.0.2] --eBGP--> [frr2 AS65001 172.30.0.3]
       |                                      |
       +-------- BMP TCP/1790 ----------------+
                          |
                   [bmp-capture 172.30.0.10]
                          |
                   samples/frr-smoke.rawbmp
```

- Private ASNs: 65000 and 65001.
- Documentation prefixes: 192.0.2.0/24 (frr1), 198.51.100.0/24 (frr2).
- BMP capture: `socat` listens on TCP/1790, writes raw bytes to
  `samples/frr-smoke.rawbmp`.

## Lab files

```
labs/frr-bmp/
  docker-compose.yml
  frr1/
    daemons          # enables zebra + bgpd
    frr.conf         # AS 65000, BMP to 172.30.0.10:1790
  frr2/
    daemons          # enables zebra + bgpd
    frr.conf         # AS 65001
```

All committed. Configuration uses `frrouting/frr:latest` image; the tag
may require adjustment for specific FRR versions.

## FRR BMP notes

BMP configuration is under `router bgp <ASN>`. The configuration commands
are accepted by FRR 8.4 but may require a newer FRR version to actually
produce BMP output.

```
bmp mirror buffer-limit 0
bmp targets <name>
bmp listener <IP> port <PORT>
```

- The `bgpd_bmp.so` module must be loaded. In this lab, `bgpd_options="-M bmp"`
  in the `daemons` file passes the `-M bmp` flag to bgpd.
- BMP support varies significantly by FRR version. FRR 8.4 parses the
  configuration but may not establish BMP connections. FRR 9.x+ is expected
  to have more complete BMP support.
- Use `show bmp` in `vtysh` to verify BMP listener and connection state.
- The `bmp` command in `vtysh` context-sensitive help (`?`) reports limited
  options in FRR 8.4, but the configuration file parser may still accept a
  wider range of commands.

## Expected message mix

Depends on FRR version and BGP session state:
- **Initiation:** sent once per BMP connection at startup
- **Peer Up:** sent when BGP session establishes
- **Route Monitoring:** prefix updates (Adj-RIB-In pre-policy by default)
- **Stats Report:** periodic, if enabled via `bmp stats-report-timer`
- **Peer Down:** sent when BGP session closes or FRR shuts down

For a minimal smoke test, Peer Up and Route Monitoring are the minimum
expected messages.

## Validation workflow

```sh
# 1. Start the lab
cd labs/frr-bmp && docker compose up -d

# 2. Wait for BGP + BMP (30–60s)
sleep 30

# 3. Verify capture size
ls -lh ../../samples/frr-smoke.rawbmp

# 4. Inspect
cargo run --bin bmpdoctor -- \
  inspect ../../samples/frr-smoke.rawbmp --summary-json

# 5. Stop
docker compose down
```

Expected pass conditions:
- Auto-detects as `raw-bmp`
- `malformed_messages == 0`
- `parse_errors == 0` (real FRR OPEN messages should parse cleanly)
- At least one Peer Up, Route Monitoring, or Initiation message
- `peers_observed >= 1`

## Troubleshooting

- **`privs_init: initial cap_set_proc failed`** — the FRR image requires
  `SYS_ADMIN` in `cap_add`. Ensure the Docker Compose file includes it.
  This is a local-lab-only concession; FRR uses it for namespace and
  routing table operations.
- **No capture file:** check `docker compose logs bmp-capture`; verify FRR
  is sending BMP by checking `docker compose logs frr1 | grep -i bmp`.
- **No BMP messages:** the FRR image may lack BMP support. Try
  `frrouting/frr:9.1` or later. Check `bgpd` startup logs.
- **BGP session not established:** check `docker compose logs frr1 | grep BGP`.
  Verify the bridge network is up and containers can reach each other.
- **Permission errors:** the `samples/` directory must be writable by the
  `bmp-capture` container (user 0:0 in alpine).

## Safety

- Generated captures go to `samples/` and remain gitignored.
- No public routes, no public peering, no host network exposure.
- Private ASNs and documentation prefixes throughout.
- No test writes to `tests/fixtures/`.

## Status

**BGP verified; BMP output not produced by FRR 10.6.1 or 8.4.**

| Component | Status |
|-----------|--------|
| Docker Compose lab | Starts cleanly |
| FRR versions tested | 8.4_git, 10.6.1_git |
| zebra / bgpd | Start (with `SYS_ADMIN`) |
| BGP session | AS65000 ↔ AS65001, confirmed via `show bgp summary` |
| `bgpd_bmp` module | Loaded and recognized (`show modules`) |
| BMP config | Accepted; `show bmp` shows target/listener entry |
| frr1 ↔ bmp-capture:1790 | TCP reachable (`nc -vz` succeeds) |
| BMP TCP connection | **Never observed** (neither direction) |
| `samples/frr-smoke.rawbmp` | **0 bytes** |

Both connection modes were tested:
- **FRR outbound** (socat listens on 1790): FRR never connects. `show bmp`
  shows the listener entry but `bgpd` never opens a TCP socket.
- **FRR inbound** (socat connects to FRR:1790): Connection refused. FRR
  does not bind a TCP listening socket for BMP.

### Conclusion

The `bgpd_bmp` module in FRR 8.4 and 10.6.1 recognizes BMP configuration
but does not establish BMP sessions in either direction. This is likely a
module-level behavior limitation, not a configuration error. The lab
infrastructure (networking, BGP, capture) is verified working.

### Next steps

Continue using RouteViews Kafka `.bmpd` and synthetic fixtures as primary
validation sources. The FRR Docker lab can be revisited if a future FRR
release resolves the BMP module behavior.
