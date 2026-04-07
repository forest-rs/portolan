# Execution Plan

## Goals

- Add a root `portolan` facade crate that gives downstream users one calmer
  entry point into the workspace.
- Keep the facade curated rather than exhaustive so it teaches one canonical
  workflow instead of flattening the crate graph blindly.
- Prove the facade by updating one real example to depend on `portolan`
  instead of importing many `portolan_*` crates directly.

## Non-Goals

- Do not turn `portolan` into a second copy of every lower-level API.
- Do not re-export all of Leit or hide backend-specific setup behind Portolan.
- Do not change crate ownership boundaries underneath the existing workspace.

## Steps

1. Add `crates/portolan` with a small curated top-level API and nested module
   re-exports for lower-level or optional crates.
2. Gate heavier facade modules such as `leit`, `schema`, `ingest`, and
   `observe` behind explicit features so the root crate does not force extra
   dependencies by default.
3. Update workspace docs and CI package lists for the new publishable crate.
4. Switch `examples/command_palette` to the facade crate to prove the preferred
   way in.
5. Run fmt, clippy, tests, and docs before committing.

## Risks

- A facade crate can become an unprincipled export barrel if the curated
  top-level path is not selective.
- Optional feature wiring can accidentally drag std or backend dependencies into
  the minimal facade path.
- The example should get simpler; if it only changes names, the facade is not
  earning its keep.
