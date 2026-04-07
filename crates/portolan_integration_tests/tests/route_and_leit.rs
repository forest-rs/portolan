// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests covering staged routing and the Leit adapter seam.

use leit_core::{FieldId, Score};
use leit_index::{InMemoryIndex, InMemoryIndexBuilder, SearchScorer};
use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
use portolan_core::{
    Affordance, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin, StandardAffordance,
};
use portolan_leit::LeitSource;
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_route::{RetrievalRouter, RoutePlan, RouteStage, StagedRetrievalSource};
use portolan_source::{CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DemoSubject(&'static str);

#[derive(Default)]
struct VecSink(Vec<PortolanHit<DemoSubject>>);

impl CandidateSink<DemoSubject> for VecSink {
    fn push(&mut self, hit: PortolanHit<DemoSubject>) {
        self.0.push(hit);
    }
}

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

struct StagedLeitSource<'a, Mapper> {
    inner: LeitSource<'a, Mapper>,
}

impl<'a, Mapper> StagedLeitSource<'a, Mapper> {
    fn new(inner: LeitSource<'a, Mapper>) -> Self {
        Self { inner }
    }
}

impl<'a, Mapper> RetrievalSource<DemoSubject> for StagedLeitSource<'a, Mapper>
where
    Mapper: Fn(u32) -> Option<DemoSubject>,
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

impl<'a, Mapper> StagedRetrievalSource<DemoSubject> for StagedLeitSource<'a, Mapper>
where
    Mapper: Fn(u32) -> Option<DemoSubject>,
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
    let leit = StagedLeitSource::new(LeitSource::new(
        &index,
        |doc_id| match doc_id {
            1 => Some(DemoSubject("command.open_scene")),
            2 => Some(DemoSubject("command.inspect_object")),
            _ => None,
        },
        SearchScorer::bm25(),
    ));
    let context = ContextSource;
    let sources: [&dyn StagedRetrievalSource<DemoSubject>; 2] = [&leit, &context];
    let query = PortolanQuery::new(
        "open",
        ParsedQuery::<(), ()>::Text {
            text: "open".into(),
        },
    );
    let router = RetrievalRouter::new();
    let mut sink = VecSink::default();

    let stats = router.retrieve_into(
        RoutePlan::standard(),
        &sources,
        &query,
        &RetrievalContext::<(), (), (), ()>::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(stats.sources_visited, 2);
    assert_eq!(stats.stages_visited, 2);
    assert_eq!(sink.0.len(), 2);
    assert_eq!(sink.0[0].subject, DemoSubject("command.open_scene"));
    assert_eq!(sink.0[0].origin, RetrievalOrigin::MaterializedIndex);
    assert_eq!(sink.0[1].subject, DemoSubject("context.recent"));
    assert_eq!(sink.0[1].origin, RetrievalOrigin::ContextCache);
}
