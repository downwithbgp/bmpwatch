# RouteViews and BMPDoctor

[RouteViews](https://www.routeviews.org/) is a public BGP data archive
operated by the University of Oregon. It collects BGP data from many
peering points worldwide and is a primary source for BGP research.

RouteViews explicitly references [RFC 7854](https://www.rfc-editor.org/rfc/rfc7854)
(BGP Monitoring Protocol) as the background specification for BMP data
collection and distribution.

## Archive data (not BMPDoctor input)

RouteViews publishes two types of archive files:

| Type            | Frequency      | Format     |
|-----------------|----------------|------------|
| RIB dumps       | Every 2 hours  | MRT (binary) |
| Update dumps    | Every 15 min   | MRT (binary) |

These are MRT-format files, not raw BMP frames. They are useful for testing
BGPKIT Parser and other BGP tooling, but are **not suitable as BMPDoctor
`raw-bmp` input**.

## Real-time data (via BGPStream)

RouteViews documents that live BMP messages are accessed through
[BGPStream](https://bgpstream.caida.org/), not by connecting directly to
RouteViews collectors.

Example command for decoded BGP event output:

```sh
bgpreader -p routeviews-stream -R rrc00 -j 65000
```

This produces decoded BGPStream/bgpreader records (PSV or JSON format), **not**
raw RFC 7854 BMP frames. The output is a decoded event stream: each line
represents a parsed BGP prefix announcement, withdrawal, or state change.

## BMPDoctor implications

### Current state

BMPDoctor's `raw-bmp` mode expects concatenated raw RFC 7854 BMP frames
(common header + payload). RouteViews archive MRT files and bgpreader PSV
output are different formats and cannot be fed directly to `bmpdoctor inspect`
or `bmpdoctor lint`.

### bgpreader PSV output (comparison use)

`bgpreader -p routeviews-stream` output can be used for **comparison
testing** against BMPDoctor output from a real BMP source. For example:

1. Capture raw BMP from a local BMP speaker (FRR/GoBGP) that peers with a
   route-views-like feed.
2. Run `bgpreader -p routeviews-stream` for the same collector and time window.
3. Compare prefix counts and timestamps between the two outputs to validate
   BMPDoctor's BGP element counting.

This is a manual, offline comparison process — no integration exists in
BMPDoctor for this workflow.

### Future modes (not implemented)

| Mode              | Input format                       | Status        |
|-------------------|------------------------------------|---------------|
| `raw-bmp`         | Raw RFC 7854 frames (.rawbmp)      | Implemented   |
| `openbmp-len`     | OpenBMP length-delimited (.obmp)   | Planned       |
| `bgpreader-psv`   | bgpreader PSV event stream         | Not planned   |

A `bgpreader-psv` mode could summarize decoded BGP event streams for
diagnostic comparison, but this would be a fundamentally different code path
from frame-level BMP parsing. It is not currently planned.
