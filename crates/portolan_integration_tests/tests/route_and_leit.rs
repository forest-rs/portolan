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
use portolan_route::{
    ReconciliationPolicy, RetrievalRouter, RoutePlan, RoutePolicy, RouteStage,
    StagedRetrievalSource, VerificationOutcome,
};
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

struct DuplicateContextSource;

impl RetrievalSource<DemoSubject> for DuplicateContextSource {
    fn retrieve_into(
        &self,
        _query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        out.push(PortolanHit {
            subject: DemoSubject("command.open_scene"),
            score: Score::new(0.20),
            evidence: Vec::new(),
            affordances: vec![Affordance::new(StandardAffordance::Inspect)],
            origin: RetrievalOrigin::ContextCache,
        });
    }
}

impl StagedRetrievalSource<DemoSubject> for DuplicateContextSource {
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

struct UniqueContextSource;

impl RetrievalSource<DemoSubject> for UniqueContextSource {
    fn retrieve_into(
        &self,
        _query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        out.push(PortolanHit {
            subject: DemoSubject("context.unique"),
            score: Score::new(0.18),
            evidence: Vec::new(),
            affordances: vec![Affordance::new(StandardAffordance::Inspect)],
            origin: RetrievalOrigin::ContextCache,
        });
    }
}

impl StagedRetrievalSource<DemoSubject> for UniqueContextSource {
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

struct BetterContextSource;

impl RetrievalSource<DemoSubject> for BetterContextSource {
    fn retrieve_into(
        &self,
        _query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        out.push(PortolanHit {
            subject: DemoSubject("command.open_scene"),
            score: Score::new(1.80),
            evidence: Vec::new(),
            affordances: vec![Affordance::new(StandardAffordance::Inspect)],
            origin: RetrievalOrigin::ContextCache,
        });
    }
}

impl StagedRetrievalSource<DemoSubject> for BetterContextSource {
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
            hit.push_evidence(Evidence::new(hit.score, ()).with_field(FieldId::new(1)));
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
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 2);
    assert_eq!(trace.stages_visited, 2);
    assert_eq!(trace.hits_emitted, 2);
    assert_eq!(trace.hits_rejected, 0);
    assert!(trace.stop_reason.is_none());
    assert_eq!(trace.stages.len(), 2);
    assert_eq!(trace.stages[0].stage, RouteStage::Materialized);
    assert_eq!(trace.stages[0].sources_visited, 1);
    assert_eq!(trace.stages[0].hits_emitted, 1);
    assert_eq!(trace.stages[1].stage, RouteStage::Contextual);
    assert_eq!(trace.stages[1].sources_visited, 1);
    assert_eq!(trace.stages[1].hits_emitted, 1);
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

#[test]
fn stops_after_stage_hit_limit_before_later_stages() {
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
        .with_enricher(|_doc_id, hit: &mut PortolanHit<DemoSubject>| {
            hit.affordances = vec![Affordance::new(StandardAffordance::Execute)];
        }),
    );
    let context = ContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 2] =
        [("leit.materialized", &leit), ("context.recent", &context)];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced_with_policy(
        RoutePlan::standard(),
        RoutePolicy {
            stop_after_stage_hits: Some(1),
            stop_after_total_hits: None,
            reconciliation_policy: ReconciliationPolicy::RetainAll,
        },
        &sources,
        &query,
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 1);
    assert_eq!(trace.stages_visited, 1);
    assert_eq!(trace.hits_emitted, 1);
    assert_eq!(trace.hits_rejected, 0);
    assert_eq!(trace.visits.len(), 1);
    assert_eq!(trace.stages.len(), 1);
    assert_eq!(trace.stages[0].stage, RouteStage::Materialized);
    assert_eq!(trace.stages[0].hits_emitted, 1);
    assert_eq!(
        trace.stop_reason,
        Some(portolan_observe::StopReason::StageHitLimitReached {
            stage: RouteStage::Materialized,
            hits_emitted: 1,
        })
    );
    assert_eq!(sink.len(), 1);
    assert_eq!(
        sink.as_slice()[0].subject,
        DemoSubject("command.open_scene")
    );
}

#[test]
fn deduplicates_subjects_across_stages_when_requested() {
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
    let duplicate_context = DuplicateContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 2] = [
        ("leit.materialized", &leit),
        ("context.duplicate", &duplicate_context),
    ];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced_with_policy(
        RoutePlan::standard(),
        RoutePolicy {
            stop_after_stage_hits: None,
            stop_after_total_hits: None,
            reconciliation_policy: ReconciliationPolicy::KeepFirstBySubject,
        },
        &sources,
        &query,
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 2);
    assert_eq!(trace.stages_visited, 2);
    assert_eq!(trace.hits_emitted, 1);
    assert_eq!(trace.duplicates_suppressed, 1);
    assert_eq!(trace.hits_replaced, 0);
    assert_eq!(trace.hits_rejected, 0);
    assert_eq!(trace.stages.len(), 2);
    assert_eq!(trace.stages[0].hits_emitted, 1);
    assert_eq!(trace.stages[0].duplicates_suppressed, 0);
    assert_eq!(trace.stages[0].hits_replaced, 0);
    assert_eq!(trace.stages[0].hits_rejected, 0);
    assert_eq!(trace.stages[1].hits_emitted, 0);
    assert_eq!(trace.stages[1].duplicates_suppressed, 1);
    assert_eq!(trace.stages[1].hits_replaced, 0);
    assert_eq!(trace.stages[1].hits_rejected, 0);
    assert!(trace.stop_reason.is_none());
    assert_eq!(sink.len(), 1);
    assert_eq!(
        sink.as_slice()[0].subject,
        DemoSubject("command.open_scene")
    );
    assert_eq!(
        sink.as_slice()[0].origin,
        RetrievalOrigin::MaterializedIndex
    );
}

#[test]
fn later_stage_can_trigger_total_hit_stop_after_empty_earlier_stage() {
    let context = ContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 1] =
        [("context.recent", &context)];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced_with_policy(
        RoutePlan::standard(),
        RoutePolicy {
            stop_after_stage_hits: None,
            stop_after_total_hits: Some(1),
            reconciliation_policy: ReconciliationPolicy::RetainAll,
        },
        &sources,
        &query,
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 1);
    assert_eq!(trace.stages_visited, 1);
    assert_eq!(trace.hits_emitted, 1);
    assert_eq!(trace.duplicates_suppressed, 0);
    assert_eq!(trace.hits_replaced, 0);
    assert_eq!(trace.hits_rejected, 0);
    assert_eq!(trace.stages[0].stage, RouteStage::Contextual);
    assert_eq!(
        trace.stop_reason,
        Some(portolan_observe::StopReason::TotalHitLimitReached {
            stage: RouteStage::Contextual,
            hits_emitted: 1,
        })
    );
    assert_eq!(sink.len(), 1);
}

#[test]
fn verification_rejects_hits_before_they_pollute_duplicate_tracking() {
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
    let duplicate_context = DuplicateContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 2] = [
        ("leit.materialized", &leit),
        ("context.duplicate", &duplicate_context),
    ];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced_verified_with_policy(
        RoutePlan::standard(),
        RoutePolicy {
            stop_after_stage_hits: None,
            stop_after_total_hits: None,
            reconciliation_policy: ReconciliationPolicy::KeepFirstBySubject,
        },
        &sources,
        &query,
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &|hit: &mut PortolanHit<DemoSubject>, _context: &RetrievalContext| {
            if hit.subject == DemoSubject("command.open_scene")
                && hit.origin == RetrievalOrigin::MaterializedIndex
            {
                VerificationOutcome::Reject
            } else {
                VerificationOutcome::Retain
            }
        },
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 2);
    assert_eq!(trace.hits_emitted, 1);
    assert_eq!(trace.duplicates_suppressed, 0);
    assert_eq!(trace.hits_replaced, 0);
    assert_eq!(trace.hits_rejected, 1);
    assert_eq!(trace.stages[0].hits_emitted, 0);
    assert_eq!(trace.stages[0].hits_replaced, 0);
    assert_eq!(trace.stages[0].hits_rejected, 1);
    assert_eq!(trace.stages[1].hits_emitted, 1);
    assert_eq!(trace.stages[1].hits_replaced, 0);
    assert_eq!(trace.stages[1].hits_rejected, 0);
    assert_eq!(sink.len(), 1);
    assert_eq!(
        sink.as_slice()[0].subject,
        DemoSubject("command.open_scene")
    );
    assert_eq!(sink.as_slice()[0].origin, RetrievalOrigin::ContextCache);
}

#[test]
fn suppressed_duplicates_do_not_count_toward_total_hit_stops() {
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
    let duplicate_context = DuplicateContextSource;
    let unique_context = UniqueContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 3] = [
        ("leit.materialized", &leit),
        ("context.duplicate", &duplicate_context),
        ("context.unique", &unique_context),
    ];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced_with_policy(
        RoutePlan::standard(),
        RoutePolicy {
            stop_after_stage_hits: None,
            stop_after_total_hits: Some(2),
            reconciliation_policy: ReconciliationPolicy::KeepFirstBySubject,
        },
        &sources,
        &query,
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 3);
    assert_eq!(trace.stages_visited, 2);
    assert_eq!(trace.hits_emitted, 2);
    assert_eq!(trace.duplicates_suppressed, 1);
    assert_eq!(trace.hits_replaced, 0);
    assert_eq!(trace.hits_rejected, 0);
    assert_eq!(trace.stages[1].hits_emitted, 1);
    assert_eq!(trace.stages[1].duplicates_suppressed, 1);
    assert_eq!(trace.stages[1].hits_replaced, 0);
    assert_eq!(
        trace.stop_reason,
        Some(portolan_observe::StopReason::TotalHitLimitReached {
            stage: RouteStage::Contextual,
            hits_emitted: 2,
        })
    );
    assert_eq!(sink.len(), 2);
    assert_eq!(sink.as_slice()[1].subject, DemoSubject("context.unique"));
}

#[test]
fn keeps_higher_scoring_subject_when_requested() {
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
    let better_context = BetterContextSource;
    let sources: [(&str, &dyn StagedRetrievalSource<DemoSubject>); 2] = [
        ("leit.materialized", &leit),
        ("context.better", &better_context),
    ];
    let query = PortolanQuery::<(), ()>::text("open");
    let router = RetrievalRouter::new();
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    let trace = router.retrieve_traced_with_policy(
        RoutePlan::standard(),
        RoutePolicy {
            stop_after_stage_hits: None,
            stop_after_total_hits: None,
            reconciliation_policy: ReconciliationPolicy::KeepBestByScore,
        },
        &sources,
        &query,
        &RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    assert_eq!(trace.sources_visited, 2);
    assert_eq!(trace.stages_visited, 2);
    assert_eq!(trace.hits_emitted, 1);
    assert_eq!(trace.duplicates_suppressed, 0);
    assert_eq!(trace.hits_replaced, 1);
    assert_eq!(trace.hits_rejected, 0);
    assert_eq!(trace.stages[0].hits_emitted, 1);
    assert_eq!(trace.stages[0].hits_replaced, 0);
    assert_eq!(trace.stages[1].hits_emitted, 0);
    assert_eq!(trace.stages[1].hits_replaced, 1);
    assert_eq!(sink.len(), 1);
    assert_eq!(
        sink.as_slice()[0].subject,
        DemoSubject("command.open_scene")
    );
    assert_eq!(sink.as_slice()[0].origin, RetrievalOrigin::ContextCache);
    assert_eq!(sink.as_slice()[0].score, Score::new(1.80));
}
