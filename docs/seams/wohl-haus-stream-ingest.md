# Seam wohl-haus/stream-ingest — draft contract

Status: draft (schema_version 0). The canonical seam block lives in
[wohl#90](https://github.com/pulseengine/wohl/issues/90); this file is the
interface anchor the block references. It becomes normative when the block
reaches `decided`, and every change to it is announced on wohl#90 before a
tag carries it.

Provider: pulseengine/wohl (wohl-hub). Consumer: haus
(avrabe/loriot-websocket-mcp — the project and binary are named haus; the
repository kept its earlier name).

## Invariants (agreed 2026-11-14, both sides)

- series identity: source + stable id + field
- stable id: node id + zone id from wohl-hub's registry, never a display name
- timestamps: hub receive time, UTC, monotonic per series; late-but-buffered
  samples keep their original hub timestamps
- invalid = absent: the consumer never sees sentinel values (0xFFFF/0x7FFF)
  or error-flagged samples; wohl's relay transformer drops them upstream
- gaps are gaps: wohl-hub never interpolates, repeats or back-fills a value
  it did not receive; the consumer renders every value with its age

## Records

### air-quality (first)

| field | type | unit |
|---|---|---|
| co2_ppm | u16 | ppm |
| pm25 | u16 | µg/m³ (descaled from the sensor's ×10 wire form upstream) |
| voc_index | i16 | index (descaled from ×10 upstream) |

A sample carries only its valid fields. Descaling happens in wohl's relay
transformer before this seam (wohl#87).

## Batch format: SenML profile (proposed; haus concurs in principle, 2026-11-14)

Batches are SenML (RFC 8428), JSON form, media type `application/senml+json`.
wohl uses a fixed subset; the consumer refuses records outside it rather than
half-supporting them.

- `bn` is exactly `wohl/<stable-id>/`. The stable id never contains `/`
  (node id and zone id are joined by `-`). `n` is the field name and never
  contains `/`. The consumer splits a resolved name into
  source = `wohl`, stable id = the middle segment, field = `n`, and refuses
  any other shape.
- `bt` plus relative `t`, UTC seconds as a float (the consumer stores ms).
- Values are numeric `v` only, with `u` from the IANA SenML units registry
  (RFC 8798 secondary units included) or documented in the record table
  above when no registry entry fits.
- wohl does NOT use: `sum`, string/boolean/data values (`vs`, `vb`, `vd`),
  base fields other than `bn`/`bt` (no `bu`, `bv`, `bs`, `but`), `ut`, or a
  `bver` other than the RFC 8428 default (10). A record carrying any of
  these is a contract violation, refused by the consumer.

## Transport

Under decision on wohl#90 (options: pull-http with cursor — recommended —
push-http, stream-sub). Replay-window retention and LAN auth are open
questions on the block; the batch format above resolves once haus confirms
on wohl#90 at ingest start.
