# Portolan

A structured retrieval substrate for navigating and acting within a live,
partially materialized world.

> A portolan was a mariner's chart: a practical map for navigation, routes,
> landmarks, and making progress through a world too large to hold all at once.

Portolan is a workspace of small crates that sits above `leit_*`.
`leit_*` owns lexical retrieval kernels and index execution.
`portolan_*` owns typed candidate retrieval over host-defined subjects, with
one explicit host-defined context snapshot, budgets, provenance, and
affordances.

The crate boundaries are intentional. Each crate owns one concern and exposes a
small public surface.

## Fence

Portolan owns typed, actionable, explainable retrieval over live system state.
It does not own the retrieval kernel, canonical world state, or UI rendering.

## `no_std` and `alloc`

The calm core is designed to work in `no_std` environments. The initial library
path enables `std` by default, but the core crates can be built with
`default-features = false` for `no_std + alloc` targets.

That applies to:

- `portolan`
- `portolan_core`
- `portolan_schema`
- `portolan_query`
- `portolan_source`

The integration-test crate is part of the workspace for cross-crate coverage.
Its tests run under `std`.

## Workspace crates

- `portolan`: curated facade crate and preferred entry point for the main retrieval workflow
- `portolan_core`: typed hits, affordances, provenance, budgets, and resolver seams
- `portolan_ingest`: materialization from projected subjects into retrieval backends
- `portolan_leit`: adapters that lower Portolan retrieval into `leit_*`
- `portolan_observe`: retrieval trace records and observation helpers
- `portolan_query`: small, host-extensible query model
- `portolan_route`: staged, multi-source retrieval orchestration
- `portolan_schema`: subject projection records and materialized field contracts
- `portolan_source`: synchronous source and sink seams for the first retrieval slice
- `portolan_integration_tests`: cross-crate integration coverage

## Examples

Examples live in top-level workspace members so core crates stay free of extra
dev-dependencies.

- `examples/basic_routing`: Leit-backed plus contextual routing over projected subjects
- `examples/command_palette`: a host-facing command palette API built on the `portolan` facade
- `examples/virtual_workset`: Leit-backed plus visible-workset virtual retrieval

## Planned crates

The intended family is broader than the first slice:

- `portolan_schema`: subject and field projection contracts
- `portolan_ingest`: incremental projection and materialization workflows
- `portolan_filter`: structured filters, facets, and metadata constraints

## Current status

This workspace now exercises the core seams end to end: host projections can be
materialized into Leit, routed alongside contextual or virtual sources, and
returned as typed hits with provenance and affordances.

The current examples and helpers demonstrate the shape of explainability, but
they do not yet guarantee backend-truth provenance for every evidence record.

Verification now has a small ergonomic surface in `portolan_route`, and the
command-palette example demonstrates composing host-truth checks without a
custom verifier type.

Retrieval context has also been simplified to one host-defined snapshot per
surface, so examples no longer need placeholder lanes for selection, focus,
view, or recents when those concepts are not all independently meaningful.

The new `portolan` crate is the preferred way into the workspace when you want
the main retrieval path without importing several `portolan_*` crates directly.

## Verification

From the workspace root:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

The library crates also support `no_std + alloc` builds with:

```bash
cargo build --workspace --exclude portolan_integration_tests --no-default-features
```

## License

Licensed under either Apache-2.0 or MIT.
