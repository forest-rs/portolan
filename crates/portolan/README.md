# portolan

Curated facade crate for Portolan structured retrieval.

This crate is the preferred way into the Portolan workspace when you want the
main retrieval workflow without importing several `portolan_*` crates directly.

Most callers should start here. If you later need narrower dependencies or more
explicit ownership boundaries, you can drop down into the lower-level
`portolan_*` crates.

Top-level exports cover the common retrieval path:

- hits, evidence, affordances, budgets, and context
- query envelopes
- live sessions and incremental search events
- candidate buffers and source traits
- staged routing, verification, and reconciliation

Lower-level or heavier layers remain under nested modules and explicit
features:

- `portolan::schema` with feature `schema`
- `portolan::ingest` with feature `ingest`
- `portolan::leit` with feature `leit`
- `portolan::live` always available for the session-based retrieval path
- `portolan::observe` always available because routed tracing is part of the
  main retrieval path

The facade is curated, not exhaustive. Lower-level crates remain available when
you want narrower dependencies or more explicit ownership boundaries.
