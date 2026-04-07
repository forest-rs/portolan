# Portolan Design

## Overview

Portolan is a structured retrieval substrate for navigating and acting within a
live, partially materialized world.

It sits between:

- canonical system state
- retrieval engines such as `leit_*`
- interaction surfaces such as palettes, pickers, inspectors, and automation

Portolan is not a UI component and not a search engine. It is the layer that
turns queries into typed, actionable, explainable candidates drawn from both
materialized and on-demand sources.

## Fence

Portolan owns typed candidate retrieval over host-defined subjects, including
query envelopes, context transport, budgets, provenance, affordance
description, and source composition. It does not own lexical indexing,
canonical world state, or host action execution.

## Invariants

- Portolan never hardcodes one global subject universe.
- Subjects remain host-defined at the edges.
- Hits carry provenance and affordances, not only scores.
- Retrieval context is explicit input, not hidden ambient state.
- Expensive work is budgeted and staged.
- Portolan may resolve affordances into host actions, but never executes those
  actions itself.

## Current crate map

- `portolan_core`
  - Owns hits, evidence, affordances, retrieval origin, budgets, context
    envelopes, and affordance-resolution seams.
  - Explicitly does not own query parsing, source orchestration, or execution.
- `portolan_query`
  - Owns the small common query envelope.
  - Explicitly does not own a rich global query language or host-specific
    semantics.
- `portolan_source`
  - Owns the first retrieval seam: sources push candidates into sinks under an
    explicit context and budget.
  - Explicitly does not own async runtimes, fusion policy, or materialization.
- `portolan_schema`
  - Owns host projection records and the materialized field contract used by
    ingest or index-building layers.
  - Explicitly does not own routing or retrieval execution.
- `portolan_route`
  - Owns staged multi-source orchestration over object-safe retrieval sources.
  - Explicitly does not own source-specific lowering or host action execution.
- `portolan_leit`
  - Owns lowering from Portolan query envelopes into `leit_*` textual retrieval.
  - Explicitly does not own routing policy or Portolan-wide query semantics.
- `portolan_integration_tests`
  - Owns cross-crate verification only.

## Planned crate map

- `portolan_ingest`
  - Incremental updates and materialization workflows.
- `portolan_filter`
  - Structured filters and faceting above the small query envelope.
- `portolan_observe`
  - Trace capture, explainability exports, and diagnostics.

## Dependency rules

- `portolan_core` depends on `leit_core` for shared score and field vocabulary.
- `portolan_schema` depends on `portolan_core` and `leit_core`.
- `portolan_query` depends only on `portolan_core`.
- `portolan_source` depends on `portolan_core` and `portolan_query`.
- `portolan_route` depends on `portolan_core`, `portolan_query`, and `portolan_source`.
- Adapter crates such as `portolan_leit` depend inward on Portolan core crates
  and outward on `leit_*`.
- Core crates must not depend on UI, async runtimes, or host application code.

## First slice

The first implementation slice should prove four things:

1. Host-defined subjects fit cleanly through the system.
2. Hits can carry evidence, affordances, and provenance without becoming
   application-specific.
3. Sources can retrieve synchronously under explicit context and budgets.
4. The seams leave room for staged or async retrieval later without forcing it
   into every trait now.

## Risks

- If the common query model grows too early, Portolan will become
  application-shaped.
- If affordances collapse into opaque strings, actionability becomes weak.
- If async is forced into the first trait boundary, the calm core becomes heavy.
- If Portolan redefines too much of Leit's retrieval vocabulary, the layering
  will become blurry.
