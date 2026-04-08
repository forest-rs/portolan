# Portolan

A structured retrieval substrate for navigating and acting within a live,
partially materialized world.

> A portolan was a mariner's chart: a practical map for navigation, routes,
> landmarks, and making progress through a world too large to hold all at once.

Portolan helps you build things like:

- command palettes
- omniboxes and jump-to dialogs
- object pickers
- inspector search
- automation surfaces that need to find and act on live things

It is the layer between:

- your application's live state
- retrieval engines such as `leit_*`
- surfaces that need ranked, actionable results

In practice, Portolan takes a query such as `"open camera"`, combines indexed
results with live contextual sources such as recents or visible objects,
verifies the surviving subjects against host truth, and returns typed hits that
say both what matched and what the user can do with it.

Portolan is a workspace of small crates that sits above `leit_*`. `leit_*`
owns lexical retrieval kernels and index execution. `portolan_*` owns typed
candidate retrieval over host-defined subjects, with one explicit host-defined
context snapshot, budgets, provenance, and affordances.

The crate boundaries are intentional. Each crate owns one concern and exposes a
small public surface.

## Fence

Portolan owns typed, actionable, explainable retrieval over live system state.
It does not own the retrieval kernel, canonical world state, or UI rendering.

## What It Does

Portolan gives you a retrieval pipeline for live applications:

1. The host projects commands, objects, settings, or documents into retrievable
   subjects.
2. Portolan queries one or more sources:
   - materialized sources such as a `leit_*` index
   - contextual sources such as recents
   - virtual sources such as a visible workset scan
3. Portolan routes, verifies, and reconciles those candidates, then returns
   typed hits with scores, provenance, evidence, and affordances.

That means a surface can ask for "camera" and receive results like:

- command `open_camera_panel`, affordance `Execute`
- object `camera.main`, affordances `Focus` and `Inspect`
- recent item `camera.debug_overlay`, affordance `Open`

Portolan is useful when not everything is fully indexed all the time and when
results need to stay connected to live host state.

## What It Is Not

- Not a UI toolkit
- Not a search engine by itself
- Not a replacement for canonical application state
- Not a command runtime

`leit_*` finds textual candidates quickly. Portolan turns those candidates,
plus host context and live sources, into results a surface can actually use.

## Getting Started

If you want the main Portolan path, start with the `portolan` facade crate.
It re-exports the common workflow types and keeps heavier integration layers
under explicit modules and features.

If you want to see the current shape end to end, start with:

- `examples/command_palette`: the clearest host-facing example
- `examples/basic_routing`: the smallest routed retrieval example
- `examples/virtual_workset`: materialized plus virtual retrieval in one pass

The smallest useful mental model is:

- define a host subject type
- build one or more retrieval sources
- route a `PortolanQuery` through them
- render the returned `PortolanHit` values
- resolve selected affordances back into host actions

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

This repository is ready to be public as an early, architecture-first Portolan
implementation. The crate graph is real, the examples are coherent, and the
workspace exercises the main seams end to end: host projections can be
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

The API should still be considered experimental. It is coherent enough for
public review and early adopters, but not yet stable enough to promise long-run
compatibility.

## How To Read This Repo

If you are new to the project:

1. Read this README for the problem statement and crate map.
2. Open `examples/command_palette` to see the most concrete host-facing flow.
3. Open `crates/portolan/src/lib.rs` for the curated facade.
4. Open `docs/design.md` for the architectural fence and glossary.

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
