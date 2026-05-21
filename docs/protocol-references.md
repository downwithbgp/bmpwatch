# Protocol References

BMPDoctor is a diagnostic/linting tool for BGP Monitoring Protocol data. This
document lists the normative and informative references that inform its
implementation.

## Normative base

| Reference | Title | Relevance |
|-----------|-------|-----------|
| [RFC 7854](https://www.rfc-editor.org/rfc/rfc7854) | BGP Monitoring Protocol (BMP) | Primary/base reference. Defines the common header, per-peer header, message types 0–6, and session lifecycle. |
| [IANA BMP Parameters](https://www.iana.org/assignments/bmp-parameters/) | BMP Message Types, Peer Types, Peer Flags, etc. | Live codepoint registry for message types, peer types, reason codes, and TLVs. |

## Extension RFCs (informative for future work)

| Reference | Title | Relevance |
|-----------|-------|-----------|
| [RFC 8671](https://www.rfc-editor.org/rfc/rfc8671) | Adj-RIB-Out Support in BMP | Defines peer flag bit 1 (L = post-policy flag) for Adj-RIB-Out route monitoring. BMPDoctor does not yet interpret Adj-RIB-Out vs Adj-RIB-In. |
| [RFC 9069](https://www.rfc-editor.org/rfc/rfc9069) | Loc-RIB Support in BMP | Defines Route Monitoring message differentiation for Loc-RIB exports. BMPDoctor's `duplicate_peer_up` rule does not yet distinguish Loc-RIB sessions from Adj-RIB sessions. |
| [RFC 9736](https://www.rfc-editor.org/rfc/rfc9736) | BMP Peer Up TLV Update | Updates Peer Up message TLVs for BGP role and mode negotiation. BMPDoctor does not parse Peer Up TLVs beyond the OPEN message in v0.1. |

## Implementation scope (v0.1)

BMPDoctor v0.1 implements and validates **only the RFC 7854 base framing and
message classification**:

- 6-byte common header (version, length, type) — RFC 7854 §3.1
- 42-byte per-peer header for message types 0, 1, 2, 3, 6 — RFC 7854 §3.2
- Message type identification 0–6 — RFC 7854 §3.3–3.8, §8
- Peer Up / Peer Down lifecycle tracking
- Timestamp ordering checks

**Not yet implemented:**

- Adj-RIB-Out vs Loc-RIB differentiation (RFC 8671, RFC 9069)
- Peer Up TLV parsing beyond the OPEN message (RFC 9736)
- Post-policy flag interpretation (RFC 8671 L flag)
- Stats Report counter interpretation (RFC 7854 §3.4)
- Initiation/Termination TLV parsing (RFC 7854 §3.7, §3.8)
- Route Mirroring TLV parsing (RFC 7854 §8)
- IANA-registered reason code labels for Peer Down
- IANA-registered peer type labels

Extension-aware interpretation is planned but deferred to post-MVP releases.
