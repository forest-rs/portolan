# portolan_leit

Leit-backed retrieval adapters for Portolan.

This crate lowers Portolan query envelopes into Leit textual retrieval and maps
Leit hits back into typed Portolan candidates through a host-supplied subject
mapper.

Use this crate when you want materialized Portolan retrieval backed by Leit.
It is the adapter layer, not the routing layer and not the projection layer.

Catalog-backed helpers can also attach projection-derived evidence and
affordances. Those helpers are convenience seams, not a claim that Portolan is
already exposing exact backend match provenance.
