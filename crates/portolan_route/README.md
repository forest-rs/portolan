# portolan_route

Staged multi-source routing for Portolan retrieval.

This crate owns simple orchestration over object-safe Portolan retrieval
sources. It executes sources in explicit stages so Portolan can stay budgeted
and incremental without forcing async into the first slice.

Route policy stays explicit. Callers choose whether to exhaust the full plan,
stop early after enough retained hits, and keep or suppress duplicate subjects
across sources.

Callers may also provide a verifier that finalizes routed hits against host
truth before they reach the caller sink.
