# portolan_core

Core types and traits for Portolan structured retrieval.

This crate owns the calm vocabulary shared across the Portolan family:
typed hits, evidence, affordances, provenance, budgets, context envelopes, and
affordance-resolution seams.

You usually do not use this crate alone. It provides the nouns that
`portolan_query`, `portolan_source`, and `portolan_route` assemble into a full
retrieval pass.
