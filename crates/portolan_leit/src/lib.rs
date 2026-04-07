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
use core::cell::RefCell;

use leit_index::{ExecutionWorkspace, InMemoryIndex, SearchScorer};
use portolan_core::{
    Evidence, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin, Score, SubjectRef,
};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_schema::{ProjectionCatalog, SubjectProjection};
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

/// Subject mapper backed by a projection catalog.
#[derive(Clone, Copy, Debug)]
pub struct CatalogSubjectMapper<'a, S: SubjectRef, A = portolan_core::StandardAffordance, M = ()> {
    catalog: &'a ProjectionCatalog<S, A, M>,
}

impl<'a, S: SubjectRef, A, M> CatalogSubjectMapper<'a, S, A, M> {
    /// Create a subject mapper over one projection catalog.
    pub const fn new(catalog: &'a ProjectionCatalog<S, A, M>) -> Self {
        Self { catalog }
    }
}

impl<S: SubjectRef, A, M> SubjectMapper<S> for CatalogSubjectMapper<'_, S, A, M> {
    fn map_subject(&self, doc_id: u32) -> Option<S> {
        self.catalog.subject(doc_id).cloned()
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

/// Builds optional evidence records for catalog-backed hits.
pub trait ProjectionEvidenceBuilder<S: SubjectRef, A, M, E> {
    /// Build one evidence record for a projected hit.
    fn build_evidence(
        &self,
        projection: &SubjectProjection<S, A, M>,
        score: Score,
    ) -> Option<Evidence<E>>;
}

impl<S, A, M, E, F> ProjectionEvidenceBuilder<S, A, M, E> for F
where
    S: SubjectRef,
    F: Fn(&SubjectProjection<S, A, M>, Score) -> Option<Evidence<E>>,
{
    fn build_evidence(
        &self,
        projection: &SubjectProjection<S, A, M>,
        score: Score,
    ) -> Option<Evidence<E>> {
        self(projection, score)
    }
}

/// Evidence builder that reports the first projected materialized field.
#[derive(Clone, Debug)]
pub struct FirstFieldEvidence<E> {
    kind: E,
}

impl<E> FirstFieldEvidence<E> {
    /// Create a first-field evidence builder with a fixed evidence kind.
    pub const fn new(kind: E) -> Self {
        Self { kind }
    }
}

impl<S, A, M, E> ProjectionEvidenceBuilder<S, A, M, E> for FirstFieldEvidence<E>
where
    S: SubjectRef,
    E: Clone,
{
    fn build_evidence(
        &self,
        projection: &SubjectProjection<S, A, M>,
        score: Score,
    ) -> Option<Evidence<E>> {
        Some(
            Evidence::new(score, self.kind.clone())
                .with_field(projection.materialized_fields.first()?.field),
        )
    }
}

/// Evidence builder that leaves catalog-backed hits unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopProjectionEvidence;

impl<S: SubjectRef, A, M, E> ProjectionEvidenceBuilder<S, A, M, E> for NoopProjectionEvidence {
    fn build_evidence(
        &self,
        _projection: &SubjectProjection<S, A, M>,
        _score: Score,
    ) -> Option<Evidence<E>> {
        None
    }
}

/// Hit enricher backed by a projection catalog.
#[derive(Clone, Copy, Debug)]
pub struct CatalogHitEnricher<
    'a,
    S: SubjectRef,
    A = portolan_core::StandardAffordance,
    M = (),
    EvidenceBuilder = NoopProjectionEvidence,
> {
    catalog: &'a ProjectionCatalog<S, A, M>,
    evidence_builder: EvidenceBuilder,
}

impl<'a, S: SubjectRef, A, M> CatalogHitEnricher<'a, S, A, M, NoopProjectionEvidence> {
    /// Create a hit enricher over one projection catalog.
    pub const fn new(catalog: &'a ProjectionCatalog<S, A, M>) -> Self {
        Self {
            catalog,
            evidence_builder: NoopProjectionEvidence,
        }
    }

    /// Attach evidence based on the first materialized field in each projection.
    pub fn with_first_field_evidence<E: Clone>(
        self,
        kind: E,
    ) -> CatalogHitEnricher<'a, S, A, M, FirstFieldEvidence<E>> {
        self.with_evidence_builder(FirstFieldEvidence::new(kind))
    }
}

impl<'a, S: SubjectRef, A, M, EvidenceBuilder> CatalogHitEnricher<'a, S, A, M, EvidenceBuilder> {
    /// Replace the evidence builder for catalog-backed hits.
    pub fn with_evidence_builder<NewEvidenceBuilder>(
        self,
        evidence_builder: NewEvidenceBuilder,
    ) -> CatalogHitEnricher<'a, S, A, M, NewEvidenceBuilder> {
        CatalogHitEnricher {
            catalog: self.catalog,
            evidence_builder,
        }
    }
}

impl<S, A, M, E, EvidenceBuilder> HitEnricher<S, A, E>
    for CatalogHitEnricher<'_, S, A, M, EvidenceBuilder>
where
    S: SubjectRef,
    A: Clone,
    EvidenceBuilder: ProjectionEvidenceBuilder<S, A, M, E>,
{
    fn enrich_hit(&self, doc_id: u32, hit: &mut PortolanHit<S, A, E>) {
        if let Some(projection) = self.catalog.projection(doc_id) {
            hit.affordances = projection.affordances.clone();
            if let Some(evidence) = self.evidence_builder.build_evidence(projection, hit.score) {
                hit.evidence.push(evidence);
            }
        }
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
                    let mut portolan_hit =
                        PortolanHit::new(subject, hit.score, RetrievalOrigin::MaterializedIndex);
                    self.enricher.enrich_hit(hit.id, &mut portolan_hit);
                    out.push(portolan_hit);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use portolan_core::{
        Affordance, FieldId, PortolanHit, RetrievalOrigin, Score, StandardAffordance,
    };
    use portolan_schema::{MaterializedField, ProjectionCatalog, SubjectProjection};

    use super::{
        CatalogHitEnricher, CatalogSubjectMapper, FirstFieldEvidence, HitEnricher, SubjectMapper,
    };

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct DemoSubject(&'static str);

    #[test]
    fn catalog_helpers_map_subjects_and_enrich_hits() {
        let catalog = ProjectionCatalog::from_projections([SubjectProjection::new(
            DemoSubject("command.open"),
            vec![MaterializedField::new(FieldId::new(1), "Open")],
        )
        .with_affordances(vec![Affordance::new(StandardAffordance::Open)])]);

        let mapper = CatalogSubjectMapper::new(&catalog);
        let mut hit = PortolanHit::new(
            mapper
                .map_subject(1)
                .expect("projection catalog should map subject"),
            Score::new(1.5),
            RetrievalOrigin::MaterializedIndex,
        );

        CatalogHitEnricher::new(&catalog)
            .with_evidence_builder(FirstFieldEvidence::new("projection"))
            .enrich_hit(1, &mut hit);

        assert_eq!(
            hit.affordances,
            vec![Affordance::new(StandardAffordance::Open)]
        );
        assert_eq!(hit.evidence.len(), 1);
        assert_eq!(hit.evidence[0].field, Some(FieldId::new(1)));
        assert_eq!(hit.evidence[0].contribution, Score::new(1.5));
        assert_eq!(hit.evidence[0].kind, "projection");
    }
}
