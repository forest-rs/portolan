# portolan_route

Staged multi-source routing for Portolan retrieval.

This crate owns simple orchestration over object-safe Portolan retrieval
sources. It executes sources in explicit stages so Portolan can stay budgeted
and incremental without forcing async into the first slice.
