# Execution Plan

## Goals

- Make the public Portolan docs navigable with intra-doc links instead of plain
  type names.
- Rewrite type-level docs so each public type explains where it fits in the
  broader retrieval workflow.
- Make it clearer how callers usually obtain or use a type when it is produced
  by another API rather than constructed directly.

## Non-Goals

- Do not change crate ownership boundaries or add new production features.
- Do not redesign the workflow APIs while documenting them.
- Do not try to make every helper type equally prominent; focus on teaching the
  main path and then situating the helper types around it.

## Steps

1. Audit the main public crates for plain-text type references and thin docs.
2. Improve the core workflow docs in `portolan_core`, `portolan_query`,
   `portolan_source`, and `portolan_route`.
3. Improve the supporting docs in `portolan_schema`, `portolan_leit`,
   `portolan_observe`, `portolan_ingest`, and the root `portolan` facade.
4. Run rustdoc, clippy, and tests so broken links or stale wording surface
   immediately.
5. Commit the docs pass once the public story feels coherent.

## Risks

- Intra-doc links can fail silently in review but loudly in `cargo doc`, so the
  validation pass matters.
- Over-documenting every small helper the same way can create noise instead of
  clarity.
- It is easy to describe future intent instead of the current implementation;
  the docs need to stay honest.
