// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests covering staged routing and the Leit adapter seam.

use leit_core::{FieldId, Score};
use leit_index::{InMemoryIndex, InMemoryIndexBuilder, SearchScorer};
use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
use portolan_core::{
    Affordance, Evidence, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin,
    StandardAffordance,
};
use portolan_leit::LeitSource;
use portolan_query::PortolanQuery;
use portolan_route::{RetrievalRouter, RoutePlan, RouteStage, StagedRetrievalSource};
use portolan_source::{CandidateBuffer, CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DemoSubject(&'static str);

struct ContextSource;

impl RetrievalSource<DemoSubject> for ContextSource {
    fn retrieve_into(
        &self,
        _query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        out.push(PortolanHit {
            subject: DemoSubject("context.recent"),
            score: Score::new(0.25),
            evidence: Vec::new(),
            affordances: vec![Affordance::new(StandardAffordance::Inspect)],
            origin: RetrievalOrigin::ContextCache,
        });
    }
}

impl StagedRetrievalSource<DemoSubject> for ContextSource {
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

struct StagedLeitSource<
    'a,
    Mapper,
    Lowerer = portolan_leit::TextQueryLowerer,
    Enricher = portolan_leit::NoopHitEnricher,
> {
    inner: LeitSource<'a, Mapper, Lowerer, Enricher>,
}

impl<'a, Mapper, Lowerer, Enricher> StagedLeitSource<'a, Mapper, Lowerer, Enricher> {
    fn new(inner: LeitSource<'a, Mapper, Lowerer, Enricher>) -> Self {
        Self { inner }
    }
}

impl<'a, Mapper, Lowerer, Enricher> RetrievalSource<DemoSubject>
    for StagedLeitSource<'a, Mapper, Lowerer, Enricher>
where
    Mapper: Fn(u32) -> Option<DemoSubject>,
    Lowerer: portolan_leit::QueryLowerer<(), ()>,
    Enricher: portolan_leit::HitEnricher<DemoSubject>,
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &RetrievalContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        self.inner.retrieve_into(query, context, budget, out);
    }
}

impl<'a, Mapper, Lowerer, Enricher> StagedRetrievalSource<DemoSubject>
    for StagedLeitSource<'a, Mapper, Lowerer, Enricher>
where
    Mapper: Fn(u32) -> Option<DemoSubject>,
    Lowerer: portolan_leit::QueryLowerer<(), ()>,
    Enricher: portolan_leit::HitEnricher<DemoSubject>,
{
    fn stage(&self) -> RouteStage {
        RouteStage::Materialized
    }
}

fn test_index() -> InMemoryIndex {
    let mut analyzers = FieldAnalyzers::new();
    let analyzer =
        Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(1), analyzer);

    let mut builder = InMemoryIndexBuilder::new(analyzers);
    builder.register_field_alias(FieldId::new(1), "title");
    builder
        .index_document(1, &[(FieldId::new(1), "open scene")])
        .expect("document should index");
    builder
        .index_document(2, &[(FieldId::new(1), "inspect object")])
        .expect("document should index");
    builder.build_index()
}

#[test]
fn routes_materialized_sources_before_contextual_sources() {
    let index = test_index();
    let leit = StagedLeitSource::new(
        LeitSource::new(
            &index,
            |doc_id| match doc_id {
                1 => Some(DemoSubject("command.open_scene")),
                2 => Some(DemoSubject("command.inspect_object")),
                _ => None,
            },
            SearchScorer::bm25(),
        )
        .with_enricher(|doc_id, hit: &mut PortolanHit<DemoSubject>| {
            hit.affordances = vec![Affordance::new(StandardAffordance::Execute)];
            hit.evidence.push(Evidence {
                field: Some(FieldId::new(1)),
                contribution: hit.score,
                kind: (),
            });
            if doc_id == 1 {
                hit.origin = RetrievalOrigin::Derived;
            }
        }),
    );
    let context = ContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 2] =
        [("leit.materialized", &leit), ("context.recent", &context)];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced(
        RoutePlan::standard(),
        &sources,
        &query,
        &RetrievalContext::<(), (), (), ()>::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 2);
    assert_eq!(trace.stages_visited, 2);
    assert_eq!(trace.visits.len(), 2);
    assert_eq!(trace.visits[0].source, "leit.materialized");
    assert_eq!(trace.visits[1].source, "context.recent");
    assert_eq!(sink.len(), 2);
    assert_eq!(
        sink.as_slice()[0].subject,
        DemoSubject("command.open_scene")
    );
    assert_eq!(sink.as_slice()[0].origin, RetrievalOrigin::Derived);
    assert_eq!(sink.as_slice()[0].affordances.len(), 1);
    assert_eq!(sink.as_slice()[0].evidence.len(), 1);
    assert_eq!(sink.as_slice()[1].subject, DemoSubject("context.recent"));
    assert_eq!(sink.as_slice()[1].origin, RetrievalOrigin::ContextCache);
}
