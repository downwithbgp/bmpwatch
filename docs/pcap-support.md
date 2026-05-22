# PCAP / PCAPNG Support

Design research note. Not implemented.

## Use case

A user captures a BMP TCP session between a router and a collector using
`tcpdump` or Wireshark and wants to inspect the BMP messages with BMPDoctor.
The capture is a `.pcap` or `.pcapng` file containing TCP segments, not a
clean byte stream.

## Technical requirements

BMP runs over TCP. Extracting BMP frames from a packet capture requires:

1. **TCP stream reassembly**: one BMP message may span multiple TCP segments;
   one TCP segment may contain multiple BMP messages. BMP has no framing
   delimiter beyond the 6-byte common header and declared message length.

2. **Flow selection**: the user must identify the BMP TCP connection (source
   port, destination port, or both). BMP collectors typically listen on
   port 1790 or an ephemeral port. Default port detection is unreliable.

3. **Edge cases**: out-of-order packets, retransmissions, missing segments,
   connection setup/teardown, and multiple concurrent BMP sessions in one
   capture all complicate reassembly.

4. **Link-layer variation**: PCAP and PCAPNG support many link types
   (Ethernet, raw IP, etc.). A parser must handle at least common ones.

## Implementation paths

### A. External-tool workflow (low effort)

Use existing tools to extract TCP payloads, then feed the resulting byte
stream to BMPDoctor:

```sh
# Extract TCP payload from a known BMP flow (collector port 1790)
tshark -r capture.pcap -Y "tcp.dstport==1790" -T fields -e tcp.payload \
  | xxd -r -p > extracted.rawbmp

# Or with tcpflow for full reassembly
tcpflow -r capture.pcap -o output_dir
cat output_dir/*.bmp-collector-ip* > reassembled.rawbmp

# Then inspect
bmpdoctor inspect reassembled.rawbmp
```

This is the recommended near-term workflow. It keeps BMPDoctor focused on
BMP byte streams and avoids TCP reassembly complexity.

**Status:** Documented, not verified against real BMP PCAP captures.

### B. Native PCAP support in BMPDoctor (medium effort)

Add `--format pcap` that:
- Parses PCAP/PCAPNG container
- Reassembles TCP streams by 4-tuple
- Extracts BMP frames using the existing `RawBmpIterator`
- Requires identifying the BMP flow (port filter or interactive selection)

This would need a Rust pcap parsing library (e.g. `pcap-file`, `pcap`,
`pcapng`) plus a TCP reassembly module. The reassembly logic is the
hard part — it must handle segment ordering, overlap, and connection
lifetime.

### C. Reassembly-only tool (low-medium effort)

A separate tool that reads PCAP, reassembles TCP streams, and writes
`.rawbmp` files. BMPDoctor then inspects those files normally. This
avoids coupling PCAP parsing to the diagnostic pipeline.

## Risks

- TCP reassembly is not a BMP problem; it's a general networking problem.
  Getting it right is hard and easy to regress.
- Ambiguous flows (multiple BMP sessions in one capture) require user input.
- PCAPNG adds complexity beyond simple PCAP (multiple interfaces, metadata).
- Scope creep: BMPDoctor is a BMP diagnostic tool, not a network protocol
  analyzer.

## Recommendation

**Defer native PCAP support.** Document the external-tool workflow as the
supported path for BMP-over-TCP captures. If user demand is high and
external tools prove insufficient, consider path C (standalone reassembly
tool) before path B (integrated `--format pcap`).

Priority: low. No implementation planned for the current development phase.
