# Execution Plan

## Goals

- Collapse `RetrievalContext` from four placeholder generic lanes into one
  host-defined context snapshot.
- Remove the current semantic mismatch where examples use `selection` to carry
  host truth and `visible_view` / `recent` to carry other unrelated state.
- Keep the new public API calm enough that examples, docs, and route helpers
  become easier to read immediately.

## Non-Goals

- Do not redesign filtering, fusion, or async retrieval in this pass.
- Do not add new production dependencies.
- Do not introduce compatibility shims for the old four-lane context API.

## Steps

1. Replace `RetrievalContext<Selection, Focus, View, Recent>` with
   `RetrievalContext<Host>` in `portolan_core`, including small constructors.
2. Thread the simplified context type through `portolan_source`,
   `portolan_route`, and `portolan_leit`.
3. Update examples and integration tests to use one explicit host snapshot per
   surface instead of split placeholder lanes.
4. Refresh migration notes, crate docs, and any doctests that still imply the
   old shape.
5. Run fmt, clippy, tests, and docs before committing.

## Risks

- This is a broad public API break, so migration notes need to be explicit.
- Example output and verifier helpers can become less clear if the new host
  snapshot names are sloppy.
- It is easy to mechanically collapse generics but miss one place where the old
  field names still leak into docs or tests.
