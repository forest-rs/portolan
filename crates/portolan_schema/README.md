# portolan_schema

Subject projection records and materialized field contracts for Portolan.

This crate defines the calm projection shape that host applications can use to
turn canonical state into typed Portolan subjects plus materialized retrieval
fields. It also provides a small projection catalog for stable doc-id assignment
and reverse lookup.

Use this crate at the boundary where host-owned data becomes retrieval-ready.
It is the natural input to `portolan_ingest` and a common companion to
`portolan_leit`.

Each subject may appear at most once in a `ProjectionCatalog`. Duplicate
subjects are rejected so document IDs and reverse lookup stay coherent.
