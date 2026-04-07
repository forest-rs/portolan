// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leit-backed retrieval adapters for Portolan.
//!
//! The first slice is intentionally narrow:
//! - textual lowering only
//! - one in-memory Leit index source
//! - host-supplied mapping from Leit document IDs to Portolan subjects

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use leit_index::{ExecutionWorkspace, InMemoryIndex, SearchScorer};
use portolan_core::{
    Affordance, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin, SubjectRef,
};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_source::{CandidateSink, RetrievalSource};

/// Maps Leit document identifiers into host-defined Portolan subjects.
pub trait SubjectMapper<S: SubjectRef> {
    /// Map a Leit document ID into a host subject.
    fn map_subject(&self, doc_id: u32) -> Option<S>;
}

impl<S, F> SubjectMapper<S> for F
where
    S: SubjectRef,
    F: Fn(u32) -> Option<S>,
{
    fn map_subject(&self, doc_id: u32) -> Option<S> {
        self(doc_id)
    }
}

/// Lowers a Portolan query into the textual query string consumed by Leit.
pub trait QueryLowerer<Scope = (), Filter = ()> {
    /// Lower a Portolan query into a textual Leit query.
    fn lower_query(&self, query: &PortolanQuery<Scope, Filter>) -> String;
}

/// Default textual lowering for the initial Portolan-Leit seam.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextQueryLowerer;

impl<Scope, Filter> QueryLowerer<Scope, Filter> for TextQueryLowerer {
    fn lower_query(&self, query: &PortolanQuery<Scope, Filter>) -> String {
        match &query.parsed {
            ParsedQuery::Text { text }
            | ParsedQuery::Scoped { text, .. }
            | ParsedQuery::Structured { text, .. } => text.clone(),
        }
    }
}

/// Enriches Portolan hits produced from Leit results.
pub trait HitEnricher<S: SubjectRef, A = portolan_core::StandardAffordance, E = ()> {
    /// Mutate a Portolan hit after the underlying Leit search returns it.
    fn enrich_hit(&self, doc_id: u32, hit: &mut PortolanHit<S, A, E>);
}

impl<S, A, E, F> HitEnricher<S, A, E> for F
where
    S: SubjectRef,
    F: Fn(u32, &mut PortolanHit<S, A, E>),
{
    fn enrich_hit(&self, doc_id: u32, hit: &mut PortolanHit<S, A, E>) {
        self(doc_id, hit);
    }
}

/// Default hit enricher that leaves hits unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopHitEnricher;

impl<S: SubjectRef, A, E> HitEnricher<S, A, E> for NoopHitEnricher {
    fn enrich_hit(&self, _doc_id: u32, _hit: &mut PortolanHit<S, A, E>) {}
}

/// Leit-backed source over one in-memory index.
#[derive(Debug)]
pub struct LeitSource<'a, Mapper, Lowerer = TextQueryLowerer, Enricher = NoopHitEnricher> {
    index: &'a InMemoryIndex,
    workspace: RefCell<ExecutionWorkspace>,
    mapper: Mapper,
    lowerer: Lowerer,
    enricher: Enricher,
    scorer: SearchScorer,
}

impl<'a, Mapper> LeitSource<'a, Mapper, TextQueryLowerer, NoopHitEnricher> {
    /// Create a new Leit-backed source with the default textual lowerer.
    pub fn new(index: &'a InMemoryIndex, mapper: Mapper, scorer: SearchScorer) -> Self {
        Self {
            index,
            workspace: RefCell::new(ExecutionWorkspace::new()),
            mapper,
            lowerer: TextQueryLowerer,
            enricher: NoopHitEnricher,
            scorer,
        }
    }
}

impl<'a, Mapper, Lowerer, Enricher> LeitSource<'a, Mapper, Lowerer, Enricher> {
    /// Replace the query lowerer.
    pub fn with_lowerer<NewLowerer>(
        self,
        lowerer: NewLowerer,
    ) -> LeitSource<'a, Mapper, NewLowerer, Enricher> {
        LeitSource {
            index: self.index,
            workspace: self.workspace,
            mapper: self.mapper,
            lowerer,
            enricher: self.enricher,
            scorer: self.scorer,
        }
    }

    /// Replace the hit enricher.
    pub fn with_enricher<NewEnricher>(
        self,
        enricher: NewEnricher,
    ) -> LeitSource<'a, Mapper, Lowerer, NewEnricher> {
        LeitSource {
            index: self.index,
            workspace: self.workspace,
            mapper: self.mapper,
            lowerer: self.lowerer,
            enricher,
            scorer: self.scorer,
        }
    }
}

impl<'a, Mapper, Lowerer, Enricher, S, Scope, Filter, Selection, Focus, View, Recent, A, E>
    RetrievalSource<S, Scope, Filter, Selection, Focus, View, Recent, A, E>
    for LeitSource<'a, Mapper, Lowerer, Enricher>
where
    S: SubjectRef,
    Mapper: SubjectMapper<S>,
    Lowerer: QueryLowerer<Scope, Filter>,
    Enricher: HitEnricher<S, A, E>,
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery<Scope, Filter>,
        _context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
    ) {
        let lowered = self.lowerer.lower_query(query);
        let limit = budget.max_candidates_per_source as usize;

        if let Ok(hits) =
            self.workspace
                .borrow_mut()
                .search(self.index, &lowered, limit, self.scorer)
        {
            for hit in hits {
                if let Some(subject) = self.mapper.map_subject(hit.id) {
                    let mut portolan_hit = PortolanHit {
                        subject,
                        score: hit.score,
                        evidence: Vec::new(),
                        affordances: Vec::<Affordance<A>>::new(),
                        origin: RetrievalOrigin::MaterializedIndex,
                    };
                    self.enricher.enrich_hit(hit.id, &mut portolan_hit);
                    out.push(portolan_hit);
                }
            }
        }
    }
}
