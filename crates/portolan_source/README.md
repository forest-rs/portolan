# portolan_source

Source and sink traits for Portolan retrieval.

This crate defines the first synchronous retrieval seam: sources receive a
query, context, and budget, then push typed candidates into a caller-provided
sink.
