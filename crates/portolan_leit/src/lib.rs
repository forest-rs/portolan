// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leit-backed retrieval adapters for Portolan.
//!
//! The first slice is intentionally narrow:
//! - textual lowering only
//! - one in-memory Leit index source
//! - host-supplied mapping from Leit document IDs to Portolan subjects
//!
//! Callers usually build a [`LeitSource`] around an [`InMemoryIndex`], provide
//! a [`SubjectMapper`] and optional [`HitEnricher`], then use it as a
//! [`RetrievalSource`] directly or through Portolan's routing layer.

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
///
/// [`LeitSource`] uses this trait after Leit returns raw document IDs so the
/// adapter can recover host subject identity and emit [`PortolanHit`] values.
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

/// Subject mapper backed by a [`ProjectionCatalog`].
///
/// Callers usually obtain this by calling [`CatalogSubjectMapper::new`] when
/// their Leit index was built from a catalog of [`SubjectProjection`] values.
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

/// Lowers a [`PortolanQuery`] into the textual query string consumed by Leit.
///
/// [`LeitSource`] asks a lowerer to turn Portolan's small query envelope into
/// the plain text shape its current backend search call expects.
pub trait QueryLowerer<Scope = (), Filter = ()> {
    /// Lower a Portolan query into a textual Leit query.
    fn lower_query(&self, query: &PortolanQuery<Scope, Filter>) -> String;
}

/// Default textual lowering for the initial Portolan-Leit seam.
///
/// Callers get this automatically through [`LeitSource::new`] unless they
/// replace it with [`LeitSource::with_lowerer`].
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

/// Enriches [`PortolanHit`] values produced from Leit results.
///
/// [`LeitSource`] constructs a minimal hit first, then gives enrichers a chance
/// to attach affordances or evidence before emitting it.
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

/// Builds optional [`Evidence`] records for catalog-backed hits.
///
/// [`CatalogHitEnricher`] uses this trait to decide what evidence, if any,
/// should be attached when one catalog projection becomes a hit.
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
///
/// This is a convenience heuristic for projection-backed hits. It does not
/// claim to identify the exact field that the retrieval backend matched.
///
/// Callers usually obtain this through
/// [`CatalogHitEnricher::with_first_field_evidence`] rather than constructing
/// it directly.
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
///
/// [`CatalogHitEnricher::new`] starts with this builder by default.
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

/// Hit enricher backed by a [`ProjectionCatalog`].
///
/// This is the main catalog-aware enrichment helper for [`LeitSource`]. Callers
/// usually construct it with [`CatalogHitEnricher::new`] and then optionally
/// add evidence behavior.
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
    ///
    /// This is the usual starting point when the Leit index was materialized
    /// from a [`ProjectionCatalog`].
    pub const fn new(catalog: &'a ProjectionCatalog<S, A, M>) -> Self {
        Self {
            catalog,
            evidence_builder: NoopProjectionEvidence,
        }
    }

    /// Attach heuristic evidence based on the first materialized field.
    ///
    /// This is useful for examples and projection-backed adapters, but it is
    /// not exact backend match provenance.
    pub fn with_first_field_evidence<E: Clone>(
        self,
        kind: E,
    ) -> CatalogHitEnricher<'a, S, A, M, FirstFieldEvidence<E>> {
        self.with_evidence_builder(FirstFieldEvidence::new(kind))
    }
}

impl<'a, S: SubjectRef, A, M, EvidenceBuilder> CatalogHitEnricher<'a, S, A, M, EvidenceBuilder> {
    /// Replace the evidence builder for catalog-backed hits.
    ///
    /// Use this when the host wants a custom evidence attachment policy rather
    /// than the default no-op or first-field helper.
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
///
/// [`LeitSource::new`] starts with this enricher by default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopHitEnricher;

impl<S: SubjectRef, A, E> HitEnricher<S, A, E> for NoopHitEnricher {
    fn enrich_hit(&self, _doc_id: u32, _hit: &mut PortolanHit<S, A, E>) {}
}

/// Leit-backed source over one in-memory index.
///
/// This is the main Portolan adapter for Leit-backed materialized retrieval.
/// Callers usually build one with [`LeitSource::new`], then optionally replace
/// the lowerer or enricher before using it as a [`RetrievalSource`] or staged
/// source.
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
    ///
    /// This is the usual constructor for materialized Portolan retrieval over
    /// a Leit [`InMemoryIndex`].
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
    ///
    /// Use this when the host wants a custom lowering strategy from
    /// [`PortolanQuery`] into Leit's textual search input.
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
    ///
    /// Use this when the host wants to attach affordances or evidence before
    /// hits leave the Leit adapter.
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

impl<'a, Mapper, Lowerer, Enricher, S, Scope, Filter, Context, A, E>
    RetrievalSource<S, Scope, Filter, Context, A, E> for LeitSource<'a, Mapper, Lowerer, Enricher>
where
    S: SubjectRef,
    Mapper: SubjectMapper<S>,
    Lowerer: QueryLowerer<Scope, Filter>,
    Enricher: HitEnricher<S, A, E>,
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery<Scope, Filter>,
        _context: &RetrievalContext<Context>,
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
        .with_affordances(vec![Affordance::new(StandardAffordance::Open)])])
        .expect("catalog should reject duplicate subjects");

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

    #[test]
    fn first_field_evidence_uses_projection_order() {
        let catalog: ProjectionCatalog<DemoSubject, StandardAffordance> =
            ProjectionCatalog::from_projections([SubjectProjection::new(
                DemoSubject("command.open"),
                vec![
                    MaterializedField::new(FieldId::new(7), "Commands"),
                    MaterializedField::new(FieldId::new(3), "Open"),
                ],
            )])
            .expect("catalog should reject duplicate subjects");

        let mapper = CatalogSubjectMapper::new(&catalog);
        let mut hit = PortolanHit::new(
            mapper
                .map_subject(1)
                .expect("projection catalog should map subject"),
            Score::new(0.75),
            RetrievalOrigin::MaterializedIndex,
        );

        CatalogHitEnricher::new(&catalog)
            .with_first_field_evidence("projection")
            .enrich_hit(1, &mut hit);

        assert_eq!(hit.evidence.len(), 1);
        assert_eq!(hit.evidence[0].field, Some(FieldId::new(7)));
        assert_eq!(hit.evidence[0].contribution, Score::new(0.75));
        assert_eq!(hit.evidence[0].kind, "projection");
    }
}
