# portolan_observe

Retrieval trace records and observation helpers for Portolan.

This crate owns generic execution trace records that higher-level routing and
diagnostics layers can populate without forcing heavyweight instrumentation into
the calm core.

Current traces can account for stage visits, retained hits, suppressed
duplicates, verification rejections, and explicit stop reasons.
