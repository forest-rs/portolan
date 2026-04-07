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

/// Leit-backed source over one in-memory index.
#[derive(Debug)]
pub struct LeitSource<'a, Mapper, Lowerer = TextQueryLowerer> {
    index: &'a InMemoryIndex,
    workspace: RefCell<ExecutionWorkspace>,
    mapper: Mapper,
    lowerer: Lowerer,
    scorer: SearchScorer,
}

impl<'a, Mapper> LeitSource<'a, Mapper, TextQueryLowerer> {
    /// Create a new Leit-backed source with the default textual lowerer.
    pub fn new(index: &'a InMemoryIndex, mapper: Mapper, scorer: SearchScorer) -> Self {
        Self {
            index,
            workspace: RefCell::new(ExecutionWorkspace::new()),
            mapper,
            lowerer: TextQueryLowerer,
            scorer,
        }
    }
}

impl<'a, Mapper, Lowerer> LeitSource<'a, Mapper, Lowerer> {
    /// Replace the query lowerer.
    pub fn with_lowerer<NewLowerer>(
        self,
        lowerer: NewLowerer,
    ) -> LeitSource<'a, Mapper, NewLowerer> {
        LeitSource {
            index: self.index,
            workspace: self.workspace,
            mapper: self.mapper,
            lowerer,
            scorer: self.scorer,
        }
    }
}

impl<'a, Mapper, Lowerer, S, Scope, Filter, Selection, Focus, View, Recent, A>
    RetrievalSource<S, Scope, Filter, Selection, Focus, View, Recent, A>
    for LeitSource<'a, Mapper, Lowerer>
where
    S: SubjectRef,
    Mapper: SubjectMapper<S>,
    Lowerer: QueryLowerer<Scope, Filter>,
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery<Scope, Filter>,
        _context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A>,
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
                    out.push(PortolanHit {
                        subject,
                        score: hit.score,
                        evidence: Vec::new(),
                        affordances: Vec::<Affordance<A>>::new(),
                        origin: RetrievalOrigin::MaterializedIndex,
                    });
                }
            }
        }
    }
}
