# portolan_query

Small query types for Portolan retrieval.

This crate keeps the common query model intentionally narrow: raw text,
optional scope, optional filters, and a parsed envelope that hosts may lower
further.

Most callers construct a `PortolanQuery` here and then pass it into sources or
the router. This crate is deliberately small so Portolan does not commit too
early to a large global query language.
