# portolan_route

Staged multi-source routing for Portolan retrieval.

This crate owns simple orchestration over object-safe Portolan retrieval
sources. It executes sources in explicit stages so Portolan can stay budgeted
and incremental without forcing async into the first slice.

This is the main entry point once a host has more than one retrieval source.
Most surfaces should feel like “query plus context plus staged sources in,
typed hits plus trace out.”

Route policy stays explicit. Callers choose whether to exhaust the full plan,
stop early after enough retained hits, and reconcile same-subject hits across
sources by retaining all of them, keeping the first, or keeping the best score.

Callers may also provide a verifier that finalizes routed hits against host
truth before they reach the caller sink.

For common host checks, `subject_verifier(...)` lets callers verify by subject
identity and explicit retrieval context without mutating the full hit. Verifier
composition stays small and explicit through `HitVerifierExt::and(...)`.

That retrieval context is now one host-defined snapshot per surface, so routing
and verification do not force callers to thread placeholder selection, focus,
or view types when one calmer host context is enough.
