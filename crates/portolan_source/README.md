# portolan_source

Source and sink traits for Portolan retrieval.

This crate defines the first synchronous retrieval seam: sources receive a
query, context, and budget, then push typed candidates into a caller-provided
sink.

It is the lowest-level retrieval boundary in Portolan. Use it when you are
implementing one source directly; use `portolan_route` when you want several
sources to run together under one plan.
