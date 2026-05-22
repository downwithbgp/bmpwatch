# FRR Local BMP Lab

Planned workflow for generating real `.rawbmp` BMP data using FRR in a
private-AS, local-only lab. **Not yet manually verified.**

## Topology

Two local FRR instances with private ASNs. This is the minimal topology
because FRR requires at least one BGP peer to emit meaningful BMP data.

```
[FRR-A AS65000] <-- BGP --> [FRR-B AS65001]
       |                          |
       +--- BMP TCP/1790 ---------+
```

- Both FRR instances emit BMP to a local listener on the same host.
- Private ASNs: 65000 and 65001.
- Loopback/container-local addresses only.
- Advertise documentation prefixes (e.g., 192.0.2.0/24, 198.51.100.0/24).

## BMP capture

FRR BMP sends raw RFC 7854 BMP frames over TCP to a collector port
(typically 1790). The simplest capture method is a TCP listener that
writes the byte stream directly to disk.

**Recommended: `socat` listener**

```sh
socat TCP-LISTEN:1790,reuseaddr,fork OPEN:samples/frr-smoke.rawbmp,creat,append
```

FRR connects to the TCP port and sends BMP frames as a clean byte stream.
`socat` writes the received bytes directly to `.rawbmp`. No reconnection
logic needed for short smoke tests.

**Alternative: `nc` listener (one-shot)**

```sh
nc -l 1790 > samples/frr-smoke.rawbmp
```

Simpler but exits after first connection closes; only one FRR instance can
use it. Use `socat` for multi-FRR setups.

**Why not BMPDoctor TCP listener yet:** No native TCP collector in MVP.
The external-tool approach keeps BMPDoctor focused on `.rawbmp` and `.bmpd`
files while still producing real test data.

## FRR configuration

### FRR-A (AS 65000)

```
router bgp 65000
 bgp router-id 10.0.0.1
 neighbor 10.0.0.2 remote-as 65001
 neighbor 10.0.0.2 bmp
 !
 address-family ipv4 unicast
  neighbor 10.0.0.2 activate
  network 192.0.2.0/24
 exit-address-family
!
bmp mirror buffer-limit 0
bmp targets lab
bmp listener 127.0.0.1 port 1790
```

### FRR-B (AS 65001)

```
router bgp 65001
 bgp router-id 10.0.0.2
 neighbor 10.0.0.1 remote-as 65000
 neighbor 10.0.0.1 bmp
 !
 address-family ipv4 unicast
  neighbor 10.0.0.1 activate
  network 198.51.100.0/24
 exit-address-family
!
bmp mirror buffer-limit 0
bmp targets lab
bmp listener 127.0.0.1 port 1790
```

Both instances point BMP to `127.0.0.1:1790`. In a container or multi-host
setup, adjust the listener address accordingly.

## Validation workflow

```sh
# 1. Start BMP capture
socat TCP-LISTEN:1790,reuseaddr,fork OPEN:samples/frr-smoke.rawbmp,creat,append &

# 2. Start FRR instances (docker, systemd, or manual)
# ... (FRR-specific startup)

# 3. Wait 30–60 seconds for BGP session establishment + BMP messages

# 4. Inspect
cargo run --bin bmpdoctor -- \
  inspect samples/frr-smoke.rawbmp --summary-json

# 5. Stop capture and FRR
kill %1  # stop socat
```

Expected pass conditions (may vary by FRR version/behavior):
- `malformed_messages == 0`
- `parse_errors == 0` (real FRR OPEN messages should parse cleanly)
- At least one Peer Up / Route Monitoring / Stats Report message
- `peers_observed >= 1`

## FRR BMP version notes

FRR BMP support is documented as stable in recent releases (8.x+). The
exact set of BMP message types emitted depends on FRR version and BGP
session state:
- **Initiation:** sent once per BMP connection at startup
- **Peer Up:** sent when BGP session establishes
- **Route Monitoring:** prefix updates (Adj-RIB-In pre-policy by default)
- **Stats Report:** periodic, if enabled via `bmp stats-report-timer`
- **Peer Down:** sent when BGP session closes or FRR shuts down

For a minimal smoke test, Peer Up and Route Monitoring are the minimum
expected messages. Initiation and Stats Report may require additional
configuration or FRR version features.

## Safety

- Generated captures go to `samples/` and remain gitignored.
- No public routes, no public peering, no host network exposure.
- Private ASNs (65000, 65001) and documentation prefixes (192.0.2.0/24,
  198.51.100.0/24) throughout.
- No test writes to `tests/fixtures/`.

## Status

**Planned — not yet manually verified.** Commands and config are provided
as a reference starting point. FRR versions, platform differences, and BMP
behavior may require adjustments during actual testing.
