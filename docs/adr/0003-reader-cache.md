# ADR 0003: Bound generation-safe reader snapshots

- Status: accepted
- Date: 2026-08-25

## Context

Polars plans the output dtype before it evaluates a plugin expression. Schema
planning and execution must therefore use the same MMDB bytes even if the file
is atomically replaced between those steps. A process-global reader cache keeps
strong byte snapshots keyed by database generation.

Canonical path, size, and modification time do not uniquely identify a file.
An updater can replace a file with different same-size contents and preserve
its modification time. Keeping every observed generation forever, however,
makes process memory grow with every database update and distinct database.

## Decision

Identify a normal filesystem generation by canonical path, byte size,
nanosecond modification time, and nanosecond metadata-change time. On Unix,
also include the device and inode numbers. On Windows, use creation time, the
volume serial number, and the complete 128-bit file identifier. These file
identities keep atomic replacements distinct even when the filesystem reports
the same timestamp tick for both files. Verify the identity before and after
reading the file. Metadata-change time on Unix also detects same-size in-place
rewrites whose modification time is restored. A same-size in-place rewrite on
Windows can retain both its creation time and file identifier, so callers must
still replace databases atomically. Avoid hashing an entire database whenever
Python constructs an expression.

Retain snapshots in process-wide insertion order up to 512 MiB by default. The
`MAXMINDDB_POLARS_CACHE_MAX_BYTES` environment variable may set a different
non-negative byte limit and is read when the first reader is opened. Always
retain the newest snapshot even when it alone exceeds the limit. Eviction drops
only the cache's strong reference; an evaluation already using the reader keeps
its own reference until it completes.

## Consequences

A planned expression normally keeps its original snapshot across replacement.
If enough newer snapshots evict that generation, later evaluation revalidates
the file. It either reopens unchanged bytes or returns an error asking the
caller to reconstruct the expression; it never substitutes a different cached
generation.

The cache is byte-bounded apart from a single oversized newest database and
readers held by active evaluations. Insertion order avoids adding a write lock
to every cache hit. Unix and Windows atomic replacements have stable file
identities; other platforms fall back to timestamps and have a weaker identity.
A future content fingerprint remains an option if another supported platform
requires it.
