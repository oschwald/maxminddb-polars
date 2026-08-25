# ADR 0002: Decode partial records with one schema-guided pass

## Status

Accepted for the `0.1` API.

## Context

A partial record can be implemented either as several independent
`decode_path` calls or as one tree lookup followed by a schema-guided record
decode. Independent expressions are simple, but repeat the search-tree
traversal for every leaf. Whole generic decoding also wastes work on fields
absent from the requested Polars Struct.

## Decision

Use the normalized `SchemaSpec` as the shared projection tree for custom
records, partial known records, and nested path outputs. The Serde seed decodes
only requested map entries, skips unrequested values in the MMDB decoder, and
recursively constructs typed Struct/List values. Partial Structs flatten to
typed leaf paths, gather each leaf as an Arrow array, and assemble the nested
Struct without per-row recursive values. A batch performs one MMDB tree lookup
per non-null valid IP and decodes each unique record offset once per selected
leaf rather than once per input row.

Keep scalar `lookup_path` on its direct typed decoder. Small and low-thread
scalar batches retain record-offset deduplication; large batches use bounded
parallel decoding, with at most 2,048 temporary decoded values per task. Keep
complete standard records on their compile-time-checked `maxminddb::geoip2`
decoders. This preserves the fastest specialized routes without creating
different schema semantics.

## Consequences

- A fused partial Struct avoids the three searches performed by three separate
  path expressions.
- Mapping and `pl.Struct` inputs normalize to the same projection tree.
- Missing declared Structs and Lists have deterministic container values;
  lookup misses remain outer nulls.
- `benchmarks/lookups.py` records partial versus independent-path throughput,
  full-record throughput, output size, CPU time, and peak process RSS.

The committed 10,000-row repeated-fixture baseline is in
`benchmarks/results/development-fixtures.json`. On that recorded environment,
the fused three-field projection took 0.78 times the scalar-path median, inside
the initial 1.30 gate. Fixture results are regression references, not forecasts
for a full production database.
