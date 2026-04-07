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
query envelopes, one explicit host-defined context snapshot, budgets,
provenance, affordance description, and source composition. It does not own
lexical indexing, canonical world state, or host action execution.

## Invariants

- Portolan never hardcodes one global subject universe.
- Subjects remain host-defined at the edges.
- Hits carry provenance and affordances, not only scores.
- Retrieval context is explicit input, not hidden ambient state.
- Portolan transports one host context snapshot rather than hard-coding
  multiple semantic lanes such as selection or focus.
- Expensive work is budgeted and staged.
- Portolan may resolve affordances into host actions, but never executes those
  actions itself.

## Glossary

- `subject`
  - A host-defined retrievable thing with stable identity, such as a command,
    object, document, setting, or trace.
- `query`
  - The Portolan retrieval request, including raw user input plus any small
    parsed structure such as scopes or filters.
- `context`
  - One explicit host-defined snapshot carried alongside a query so retrieval
    can depend on live state without reading hidden ambient globals.
- `source`
  - A producer of retrieval candidates. Sources may be materialized,
    contextual, or virtual.
- `hit`
  - A typed candidate emitted by retrieval, including subject identity, score,
    origin, evidence, and affordances.
- `evidence`
  - Structured explanation attached to a hit describing why it matched or where
    its score came from.
- `affordance`
  - A structured description of what a host or surface can do with one hit,
    such as open, inspect, focus, or execute.
- `origin`
  - Provenance for how a hit entered the pipeline, such as materialized index,
    context cache, visible workset, virtual scan, or derived result.
- `route`
  - The staged execution of multiple sources under one plan, policy, query, and
    context.
- `verification`
  - A host-owned finalization step that retains or rejects routed hits against
    canonical truth before they reach the caller sink.
- `reconciliation`
  - The policy Portolan applies when multiple sources emit the same subject,
    such as retaining all hits, keeping the first, or keeping the best score.
- `projection`
  - A host-authored materialized view of one subject used to feed retrieval
    backends such as Leit.

## Current crate map

- `portolan`
  - Owns the curated facade and canonical entry path into the Portolan family.
  - Explicitly does not replace the ownership boundaries of the lower-level crates.
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
- `portolan_ingest`
  - Owns projection-to-index materialization workflows.
  - Explicitly does not own query execution or routing.
- `portolan_observe`
  - Owns generic trace records for retrieval execution and later diagnostics.
  - Explicitly does not own routing policy or source execution.
- `portolan_schema`
  - Owns host projection records and the materialized field contract used by
    ingest or index-building layers.
  - Explicitly does not own routing or retrieval execution.
- `portolan_route`
  - Owns staged multi-source orchestration over object-safe retrieval sources.
  - Explicitly does not own source-specific lowering or host action execution.
- `portolan_leit`
  - Owns lowering from Portolan query envelopes into `leit_*` textual retrieval
    plus adapter seams from projection catalogs back into typed Portolan hits.
  - Explicitly does not own routing policy or Portolan-wide query semantics.
- `portolan_integration_tests`
  - Owns cross-crate verification only.

## Planned crate map

- `portolan_ingest`
  - Incremental updates and materialization workflows.
- `portolan_filter`
  - Structured filters and faceting above the small query envelope.

## Dependency rules

- `portolan` depends inward on the main Portolan crates and may expose
  optional modules for heavier layers such as schema, ingest, and Leit-backed
  retrieval.
- `portolan_core` depends on `leit_core` for shared score and field vocabulary.
- `portolan_schema` depends on `portolan_core` and `leit_core`.
- `portolan_query` depends only on `portolan_core`.
- `portolan_source` depends on `portolan_core` and `portolan_query`.
- `portolan_ingest` depends on `portolan_schema` and specific backend crates such as `leit_*`.
- `portolan_observe` depends on `portolan_core`.
- `portolan_route` depends on `portolan_core`, `portolan_observe`, `portolan_query`, and `portolan_source`.
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
- If helper evidence is mistaken for true backend match provenance, Portolan
  will overclaim explainability before the lower layers can actually support it.
- If affordances collapse into opaque strings, actionability becomes weak.
- If async is forced into the first trait boundary, the calm core becomes heavy.
- If Portolan redefines too much of Leit's retrieval vocabulary, the layering
  will become blurry.
