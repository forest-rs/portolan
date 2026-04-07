# command_palette

Demonstrates a simple host-facing command palette API built on the `portolan`
facade crate, including retrieval, routing, provenance, affordance
resolution, and host-truth verification before rendering.

The example now composes verification from a subject-level helper plus one
additional check, rather than defining a bespoke verifier type.

It also packages palette truth, visible objects, and recents into one
`PaletteHost` snapshot, which matches Portolan's calmer single-context API.
