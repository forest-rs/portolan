# portolan_ingest

Projection-to-index materialization for Portolan.

This crate turns `portolan_schema` projection catalogs into backend-specific
retrieval artifacts. The first slice targets Leit's in-memory index builder.

Today that means “materialize a projection catalog into a Leit index.” Over
time this crate is also the natural home for more incremental materialization
workflows.
