# portolan_ingest

Projection-to-index materialization for Portolan.

This crate turns `portolan_schema` projection catalogs into backend-specific
retrieval artifacts. The first slice targets Leit's in-memory index builder.
