// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example showing Portolan routing over projected subjects.

use leit_core::FieldId;
use leit_index::{InMemoryIndex, InMemoryIndexBuilder, SearchScorer};
use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
use portolan_core::{
    Affordance, Evidence, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin,
    StandardAffordance,
};
use portolan_leit::{LeitSource, TextQueryLowerer};
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

#[derive(Clone, Debug)]
struct CommandMetadata {
    title: &'static str,
}

type DemoHit = PortolanHit<DemoSubject, StandardAffordance, &'static str>;
type SubjectMapper = Box<dyn Fn(u32) -> Option<DemoSubject>>;
type HitEnricher = Box<dyn Fn(u32, &mut DemoHit)>;
type DemoLeitSource<'a> = LeitSource<'a, SubjectMapper, TextQueryLowerer, HitEnricher>;

struct CommandProjector;

impl ProjectSubject<CommandRecord, DemoSubject, StandardAffordance, CommandMetadata>
    for CommandProjector
{
    fn project(
        &self,
        value: &CommandRecord,
    ) -> SubjectProjection<DemoSubject, StandardAffordance, CommandMetadata> {
        SubjectProjection::new(
            DemoSubject::Command(value.id),
            vec![
                MaterializedField::new(FieldId::new(1), value.title),
                MaterializedField::new(FieldId::new(2), value.description),
            ],
        )
        .with_affordances(vec![Affordance::new(StandardAffordance::Execute)])
        .with_metadata(CommandMetadata { title: value.title })
    }
}

#[derive(Default)]
struct VecSink(Vec<DemoHit>);

impl CandidateSink<DemoSubject, StandardAffordance, &'static str> for VecSink {
    fn push(&mut self, hit: DemoHit) {
        self.0.push(hit);
    }
}

struct ContextSource;

impl RetrievalSource<DemoSubject, (), (), (), (), (), (), StandardAffordance, &'static str>
    for ContextSource
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject, StandardAffordance, &'static str>,
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

impl StagedRetrievalSource<DemoSubject, (), (), (), (), (), (), StandardAffordance, &'static str>
    for ContextSource
{
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

struct MaterializedSource<'a> {
    inner: DemoLeitSource<'a>,
}

impl<'a> MaterializedSource<'a> {
    fn new(inner: DemoLeitSource<'a>) -> Self {
        Self { inner }
    }
}

impl RetrievalSource<DemoSubject, (), (), (), (), (), (), StandardAffordance, &'static str>
    for MaterializedSource<'_>
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &RetrievalContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject, StandardAffordance, &'static str>,
    ) {
        self.inner.retrieve_into(query, context, budget, out);
    }
}

impl StagedRetrievalSource<DemoSubject, (), (), (), (), (), (), StandardAffordance, &'static str>
    for MaterializedSource<'_>
{
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

fn affordance_label(affordance: StandardAffordance) -> &'static str {
    match affordance {
        StandardAffordance::Execute => "execute",
        StandardAffordance::Open => "open",
        StandardAffordance::Focus => "focus",
        StandardAffordance::Inspect => "inspect",
        StandardAffordance::Reveal => "reveal",
        StandardAffordance::Toggle => "toggle",
        StandardAffordance::Preview => "preview",
        StandardAffordance::RefineQuery => "refine_query",
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

fn build_index(
    projections: &[SubjectProjection<DemoSubject, StandardAffordance, CommandMetadata>],
) -> InMemoryIndex {
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
    let projections: Vec<SubjectProjection<DemoSubject, StandardAffordance, CommandMetadata>> =
        records
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
        println!(
            "     affordances: {}",
            projection
                .affordances
                .iter()
                .map(|affordance| affordance_label(affordance.action))
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("     metadata title: {}", projection.metadata.title);
    }
    println!();

    let index = build_index(&projections);
    let subjects: Vec<_> = projections
        .iter()
        .map(|projection| projection.subject.clone())
        .collect();
    let projected_affordances: Vec<_> = projections
        .iter()
        .map(|projection| projection.affordances.clone())
        .collect();
    let projected_titles: Vec<_> = projections
        .iter()
        .map(|projection| projection.metadata.title)
        .collect();
    let mapper: SubjectMapper =
        Box::new(move |doc_id: u32| subjects.get((doc_id as usize).saturating_sub(1)).cloned());
    let enricher: HitEnricher = Box::new(move |doc_id: u32, hit: &mut DemoHit| {
        let index = (doc_id as usize).saturating_sub(1);
        if let Some(affordances) = projected_affordances.get(index) {
            hit.affordances = affordances.clone();
        }
        if projected_titles.get(index).is_some() {
            hit.evidence.push(Evidence {
                field: Some(FieldId::new(1)),
                contribution: hit.score,
                kind: "title_projection",
            });
        }
    });
    let materialized = MaterializedSource::new(
        LeitSource::new(&index, mapper, SearchScorer::bm25()).with_enricher(enricher),
    );
    let contextual = ContextSource;
    let sources: [&dyn StagedRetrievalSource<
        DemoSubject,
        (),
        (),
        (),
        (),
        (),
        (),
        StandardAffordance,
        &'static str,
    >; 2] = [&materialized, &contextual];
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
        if !hit.affordances.is_empty() {
            println!(
                "      affordances: {}",
                hit.affordances
                    .iter()
                    .map(|affordance| affordance_label(affordance.action))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !hit.evidence.is_empty() {
            for evidence in &hit.evidence {
                println!(
                    "      evidence: field={:?} contribution={:.3} kind={}",
                    evidence.field.map(FieldId::as_u32),
                    evidence.contribution.as_f32(),
                    evidence.kind
                );
            }
        }
    }
}
