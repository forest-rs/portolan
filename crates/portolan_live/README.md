# portolan_live

Live query sessions and incremental search updates for Portolan.

This crate adds a session-based retrieval protocol on top of the calm one-shot
`portolan_source` seam. It is for sources that need to:

- stream partial results
- revise or retract earlier results
- report progress or lifecycle changes
- remain tied to one live query session over time

It also includes a small staged coordinator that can normalize several live or
lifted snapshot sources into one event stream.

When a coordinated session is canceled, sources that advertise cancellation are
reported as `Canceled`. Sources that do not are marked `Stale` so the
coordinator can stop observing them without claiming stronger guarantees than
the source provides.

Cancellation is cooperative. A source that sets `can_cancel = true` is expected
to stop background work promptly, avoid emitting further non-terminal events,
and converge on a terminal canceled state when polled directly after
cancellation. The coordinator's guarantee is narrower: it records the outcome
in the coordinated event stream and stops observing that source after the
terminal status is emitted. It does not prove that source-side work truly
halted.

`SnapshotLiveSource` is the bridge for existing one-shot sources. In this first
slice, it wraps cloneable `'static` sources and lifts them into one-shot live
sessions. Successful runs emit `Begin`, `ApplyPatch`, and terminal
`StatusChanged` events. Sessions canceled before retrieval emit `Begin` and a
terminal canceled status instead.
