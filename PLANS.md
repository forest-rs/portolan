# Execution Plan

## Goals

- Add a first-class verification seam to routed retrieval so Portolan can drop
  or mutate candidates before they reach the caller sink.
- Prove the current reconciliation story explicitly in tests instead of
  implying more than the implementation does.
- Harden the virtual-workset example against missing host state.

## Non-Goals

- Do not design `portolan_filter` before the incoming Leit filter work lands.
- Do not introduce async or a broad fusion/reranking subsystem in this pass.
- Do not turn examples into production surface crates.

## Steps

1. Add an optional route-level hit verifier plus trace/stats accounting for
   rejected hits.
2. Add regression tests for verification, dedup-plus-stop interaction, and the
   current “keep first by subject” reconciliation behavior.
3. Remove the remaining stale-state panic from the virtual-workset example and
   add a focused test for that path.
4. Update docs and migration notes for any public API additions.

## Risks

- Verifier traits can get too generic too quickly if they try to own all future
  finalization logic.
- Route stop semantics must remain explicit once duplicate suppression and
  verification rejections both exist.
- It is easy to accidentally call simple dedup “fusion”; the docs need to stay
  calibrated.
