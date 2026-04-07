// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example showing Portolan routing over projected subjects.

use leit_core::FieldId;
use leit_index::{InMemoryIndex, InMemoryIndexBuilder, SearchScorer};
use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
use portolan_core::{
    Affordance, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin, StandardAffordance,
};
use portolan_leit::LeitSource;
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_route::{RetrievalRouter, RoutePlan, RouteStage, StagedRetrievalSource};
use portolan_schema::{MaterializedField, ProjectSubject, SubjectProjection};
use portolan_source::{CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DemoSubject {
    Command(&'static str),
    Recent(&'static str),
}

#[derive(Clone, Debug)]
struct CommandRecord {
    id: &'static str,
    title: &'static str,
    description: &'static str,
}

struct CommandProjector;

impl ProjectSubject<CommandRecord, DemoSubject> for CommandProjector {
    fn project(&self, value: &CommandRecord) -> SubjectProjection<DemoSubject> {
        SubjectProjection::new(
            DemoSubject::Command(value.id),
            vec![
                MaterializedField::new(FieldId::new(1), value.title),
                MaterializedField::new(FieldId::new(2), value.description),
            ],
        )
        .with_affordances(vec![Affordance::new(StandardAffordance::Execute)])
    }
}

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
        query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        let should_match = match &query.parsed {
            ParsedQuery::Text { text }
            | ParsedQuery::Scoped { text, .. }
            | ParsedQuery::Structured { text, .. } => text.contains("open"),
        };

        if should_match {
            out.push(PortolanHit {
                subject: DemoSubject::Recent("recent.scene"),
                score: leit_core::Score::new(0.2),
                evidence: Vec::new(),
                affordances: vec![Affordance::new(StandardAffordance::Open)],
                origin: RetrievalOrigin::ContextCache,
            });
        }
    }
}

impl StagedRetrievalSource<DemoSubject> for ContextSource {
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

struct MaterializedSource<'a> {
    inner: LeitSource<'a, Box<dyn Fn(u32) -> Option<DemoSubject>>>,
}

impl<'a> MaterializedSource<'a> {
    fn new(inner: LeitSource<'a, Box<dyn Fn(u32) -> Option<DemoSubject>>>) -> Self {
        Self { inner }
    }
}

impl RetrievalSource<DemoSubject> for MaterializedSource<'_> {
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

impl StagedRetrievalSource<DemoSubject> for MaterializedSource<'_> {
    fn stage(&self) -> RouteStage {
        RouteStage::Materialized
    }
}

fn subject_label(subject: &DemoSubject) -> &'static str {
    match subject {
        DemoSubject::Command(id) | DemoSubject::Recent(id) => id,
    }
}

fn stage_label(stage: RouteStage) -> &'static str {
    match stage {
        RouteStage::Materialized => "materialized",
        RouteStage::Contextual => "contextual",
        RouteStage::Virtual => "virtual",
    }
}

fn origin_label(origin: RetrievalOrigin) -> &'static str {
    match origin {
        RetrievalOrigin::MaterializedIndex => "materialized_index",
        RetrievalOrigin::ContextCache => "context_cache",
        RetrievalOrigin::VisibleWorkset => "visible_workset",
        RetrievalOrigin::VirtualScan => "virtual_scan",
        RetrievalOrigin::Derived => "derived",
    }
}

fn analyzers() -> FieldAnalyzers {
    let mut analyzers = FieldAnalyzers::new();
    let analyzer =
        Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(1), analyzer);
    let analyzer =
        Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(2), analyzer);
    analyzers
}

fn build_index(projections: &[SubjectProjection<DemoSubject>]) -> InMemoryIndex {
    let mut builder = InMemoryIndexBuilder::new(analyzers());
    builder.register_field_alias(FieldId::new(1), "title");
    builder.register_field_alias(FieldId::new(2), "description");

    for (doc_id, projection) in projections.iter().enumerate() {
        let mut fields = Vec::new();
        for field in &projection.materialized_fields {
            fields.push((field.field, field.text.as_str()));
        }
        let doc_id = u32::try_from(doc_id + 1).expect("example projection count should fit in u32");
        builder
            .index_document(doc_id, &fields)
            .expect("projection should index");
    }

    builder.build_index()
}

fn main() {
    let records = [
        CommandRecord {
            id: "command.open_scene",
            title: "Open Scene",
            description: "Open the current scene in the active editor",
        },
        CommandRecord {
            id: "command.inspect_selection",
            title: "Inspect Selection",
            description: "Inspect the currently selected object",
        },
    ];
    let projector = CommandProjector;
    let projections: Vec<_> = records
        .iter()
        .map(|record| projector.project(record))
        .collect();

    println!("Portolan basic routing example");
    println!();
    println!("1. Project host records into retrievable subjects");
    for projection in &projections {
        println!("   - subject: {}", subject_label(&projection.subject));
        for field in &projection.materialized_fields {
            println!("     field {} => {}", field.field.as_u32(), field.text);
        }
        println!("     affordances: {}", projection.affordances.len());
    }
    println!();

    let index = build_index(&projections);
    let subjects: Vec<_> = projections
        .iter()
        .map(|projection| projection.subject.clone())
        .collect();
    let mapper = move |doc_id: u32| subjects.get((doc_id as usize).saturating_sub(1)).cloned();
    let materialized = MaterializedSource::new(LeitSource::new(
        &index,
        Box::new(mapper),
        SearchScorer::bm25(),
    ));
    let contextual = ContextSource;
    let sources: [&dyn StagedRetrievalSource<DemoSubject>; 2] = [&materialized, &contextual];
    let router = RetrievalRouter::new();
    let plan = RoutePlan::standard();
    let query = PortolanQuery::new(
        "open",
        ParsedQuery::<(), ()>::Text {
            text: "open".into(),
        },
    );
    let mut sink = VecSink::default();

    println!("2. Build a Leit-backed materialized source and one contextual source");
    println!("   - source 1 stage: {}", stage_label(materialized.stage()));
    println!("   - source 2 stage: {}", stage_label(contextual.stage()));
    println!();
    println!("3. Route query {:?}", query.raw);
    println!(
        "   - stage order: {} -> {} -> {}",
        stage_label(plan.stages()[0]),
        stage_label(plan.stages()[1]),
        stage_label(plan.stages()[2])
    );
    println!(
        "   - budget: {} candidates/source, {} virtual expansions, {} nodes, {}us",
        RetrievalBudget::interactive_default().max_candidates_per_source,
        RetrievalBudget::interactive_default().max_virtual_expansions,
        RetrievalBudget::interactive_default().max_nodes_scanned,
        RetrievalBudget::interactive_default().max_time_us
    );
    println!();

    let stats = router.retrieve_into(
        plan,
        &sources,
        &query,
        &RetrievalContext::<(), (), (), ()>::default(),
        RetrievalBudget::interactive_default(),
        &mut sink,
    );

    println!("4. Results");
    println!(
        "   visited {} stages across {} sources",
        stats.stages_visited, stats.sources_visited
    );
    for (index, hit) in sink.0.iter().enumerate() {
        println!(
            "   {}. {} | score={:.3} | origin={}",
            index + 1,
            subject_label(&hit.subject),
            hit.score.as_f32(),
            origin_label(hit.origin)
        );
    }
}
