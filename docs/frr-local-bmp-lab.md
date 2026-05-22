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

BMP configuration is under `router bgp <ASN>`:

```
bmp mirror buffer-limit 0
bmp targets <name>
bmp listener <IP> port <PORT>
```

- The `bmp` command syntax may vary by FRR version.
- BMP support is built into `bgpd`; no separate daemon required.
- If `bmp` commands are not recognized, ensure the FRR image includes BMP
  support and check the `bgpd` logs for module loading errors.
- If BMP target by hostname does not work, use the IP address
  (`172.30.0.10`) as shown in the config.

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

**Planned — not yet manually verified.** Lab files are committed as
reference; commands and config may require adjustments during actual testing.
