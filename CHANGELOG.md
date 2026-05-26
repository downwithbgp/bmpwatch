# Changelog

## v0.1.1

Parser hardening and doctor diagnostics hotfix.

- Prevent malformed Statistics Report messages from panicking the parser.
- Reject oversized `.bmpd` payload lengths before allocation.
- Avoid false-positive doctor parse errors by only validating route-bearing BMP messages with bgpkit-parser.
- Report truncated Peer Down Notifications that omit the mandatory reason byte.
- Add regression coverage for malformed BMP/OpenBMP inputs.
