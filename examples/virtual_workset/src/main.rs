// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example showing Portolan routing across materialized and virtual sources.

use std::string::String;
use std::vec::Vec;

use leit_core::{FieldId, Score};
use leit_index::SearchScorer;
use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
use portolan_core::{
    Affordance, Evidence, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin,
    StandardAffordance,
};
use portolan_ingest::{FieldAlias, build_leit_index};
use portolan_leit::{CatalogHitEnricher, CatalogSubjectMapper, LeitSource};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_route::{RetrievalRouter, RoutePlan, RouteStage, StagedRetrievalSource};
use portolan_schema::{MaterializedField, ProjectSubject, ProjectionCatalog, SubjectProjection};
use portolan_source::{CandidateBuffer, CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DemoSubject {
    Command(&'static str),
    Entity(&'static str),
}

#[derive(Clone, Debug)]
struct CommandRecord {
    id: &'static str,
    title: &'static str,
    description: &'static str,
}

#[derive(Clone, Debug)]
struct CommandMetadata {
    summary: &'static str,
}

#[derive(Clone, Debug)]
struct VisibleEntity {
    id: &'static str,
    label: &'static str,
}

#[derive(Clone, Debug)]
struct VisibleWorkset {
    entities: Vec<VisibleEntity>,
}

type DemoContext = RetrievalContext<VisibleWorkset>;
type DemoEvidence = &'static str;

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
        .with_affordances(vec![Affordance::new(StandardAffordance::Open)])
        .with_metadata(CommandMetadata {
            summary: value.description,
        })
    }
}

struct VisibleWorksetSource;

impl RetrievalSource<DemoSubject, (), (), VisibleWorkset, StandardAffordance, DemoEvidence>
    for VisibleWorksetSource
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &DemoContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject, StandardAffordance, DemoEvidence>,
    ) {
        let text = match &query.parsed {
            ParsedQuery::Text { text }
            | ParsedQuery::Scoped { text, .. }
            | ParsedQuery::Structured { text, .. } => text.to_ascii_lowercase(),
        };

        let Some(workset) = context.host.as_ref() else {
            return;
        };

        let limit = usize::try_from(budget.max_virtual_expansions)
            .expect("virtual expansion budget should fit in usize");
        let mut matches = 0_usize;

        for entity in &workset.entities {
            if matches >= limit {
                break;
            }

            if !entity.label.to_ascii_lowercase().contains(&text) {
                continue;
            }

            matches += 1;
            out.push(
                PortolanHit::new(
                    DemoSubject::Entity(entity.id),
                    Score::new(0.35),
                    RetrievalOrigin::VirtualScan,
                )
                .with_evidence(vec![Evidence::new(Score::new(0.35), "visible_label_scan")])
                .with_affordances(vec![
                    Affordance::new(StandardAffordance::Focus),
                    Affordance::new(StandardAffordance::Inspect),
                ]),
            );
        }
    }
}

impl StagedRetrievalSource<DemoSubject, (), (), VisibleWorkset, StandardAffordance, DemoEvidence>
    for VisibleWorksetSource
{
    fn stage(&self) -> RouteStage {
        RouteStage::Virtual
    }
}

struct MaterializedSource<Inner> {
    inner: Inner,
}

impl<Inner> MaterializedSource<Inner> {
    fn new(inner: Inner) -> Self {
        Self { inner }
    }
}

impl<Inner> RetrievalSource<DemoSubject, (), (), VisibleWorkset, StandardAffordance, DemoEvidence>
    for MaterializedSource<Inner>
where
    Inner: RetrievalSource<DemoSubject, (), (), VisibleWorkset, StandardAffordance, DemoEvidence>,
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &DemoContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject, StandardAffordance, DemoEvidence>,
    ) {
        self.inner.retrieve_into(query, context, budget, out);
    }
}

impl<Inner>
    StagedRetrievalSource<DemoSubject, (), (), VisibleWorkset, StandardAffordance, DemoEvidence>
    for MaterializedSource<Inner>
where
    Inner: RetrievalSource<DemoSubject, (), (), VisibleWorkset, StandardAffordance, DemoEvidence>,
{
    fn stage(&self) -> RouteStage {
        RouteStage::Materialized
    }
}

fn analyzers() -> FieldAnalyzers {
    let mut analyzers = FieldAnalyzers::new();
    let title = Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(1), title);
    let description =
        Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(2), description);
    analyzers
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

fn subject_label(subject: &DemoSubject) -> &'static str {
    match subject {
        DemoSubject::Command(id) | DemoSubject::Entity(id) => id,
    }
}

fn visible_entity_lines(context: &DemoContext) -> Vec<String> {
    match &context.host {
        Some(workset) => workset
            .entities
            .iter()
            .map(|entity| format!("{} ({})", entity.id, entity.label))
            .collect(),
        None => vec!["[stale] visible workset is unavailable in host state".to_owned()],
    }
}

fn main() {
    let records = [
        CommandRecord {
            id: "command.open_camera_panel",
            title: "Open Camera Panel",
            description: "Open camera controls in the inspector",
        },
        CommandRecord {
            id: "command.center_selection",
            title: "Center Selection",
            description: "Center the camera on the current selection",
        },
    ];
    let projector = CommandProjector;
    let catalog =
        ProjectionCatalog::from_projections(records.iter().map(|record| projector.project(record)))
            .expect("example should not contain duplicate subjects");
    let index = build_leit_index(
        &catalog,
        analyzers(),
        &[
            FieldAlias::new(FieldId::new(1), "title"),
            FieldAlias::new(FieldId::new(2), "description"),
        ],
    )
    .expect("catalog should materialize into a Leit index");
    let materialized = MaterializedSource::new(
        LeitSource::new(
            &index,
            CatalogSubjectMapper::new(&catalog),
            SearchScorer::bm25(),
        )
        .with_enricher(
            CatalogHitEnricher::new(&catalog).with_first_field_evidence("materialized_projection"),
        ),
    );
    let virtual_workset = VisibleWorksetSource;
    let sources = [
        ("leit.materialized", &materialized as _),
        ("visible.workset", &virtual_workset as _),
    ];
    let router = RetrievalRouter::new();
    let plan = RoutePlan::standard();
    let query = PortolanQuery::<(), ()>::text("camera");
    let budget = RetrievalBudget {
        max_candidates_per_source: 64,
        max_virtual_expansions: 2,
        max_nodes_scanned: 256,
        max_time_us: 5_000,
    };
    let context = RetrievalContext::with_host(VisibleWorkset {
        entities: vec![
            VisibleEntity {
                id: "entity.camera.main",
                label: "Main Camera",
            },
            VisibleEntity {
                id: "entity.camera.preview",
                label: "Preview Camera",
            },
            VisibleEntity {
                id: "entity.light.key",
                label: "Key Light",
            },
        ],
    });
    let mut sink = CandidateBuffer::<DemoSubject, StandardAffordance, DemoEvidence>::new();

    println!("Portolan virtual workset example");
    println!();
    println!("1. Materialize stable command projections into a Leit index");
    for (_, projection) in catalog.iter() {
        println!("   - subject: {}", subject_label(&projection.subject));
        println!("     summary: {}", projection.metadata.summary);
    }
    println!();
    println!("2. Route one query across materialized and virtual sources");
    println!(
        "   - stage order: {} -> {} -> {}",
        stage_label(plan.stages()[0]),
        stage_label(plan.stages()[1]),
        stage_label(plan.stages()[2])
    );
    println!(
        "   - virtual budget: {} expansions",
        budget.max_virtual_expansions
    );
    println!("   - visible entities:");
    for line in visible_entity_lines(&context) {
        println!("     - {line}");
    }
    println!();

    let trace = router.retrieve_traced(plan, &sources, &query, &context, budget, &mut sink);

    println!("3. Results for {:?}", query.raw);
    println!(
        "   visited {} stages across {} sources",
        trace.stages_visited, trace.sources_visited
    );
    println!("   trace:");
    for visit in &trace.visits {
        println!(
            "     - stage={} source={}",
            stage_label(visit.stage),
            visit.source
        );
    }
    for (index, hit) in sink.as_slice().iter().enumerate() {
        println!(
            "   {}. {} | score={:.3} | origin={}",
            index + 1,
            subject_label(&hit.subject),
            hit.score.as_f32(),
            origin_label(hit.origin)
        );
        println!(
            "      affordances: {}",
            hit.affordances
                .iter()
                .map(|affordance| affordance_label(affordance.action))
                .collect::<Vec<_>>()
                .join(", ")
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_missing_visible_workset_without_panicking() {
        let context = RetrievalContext::<VisibleWorkset>::default();

        let lines = visible_entity_lines(&context);

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            "[stale] visible workset is unavailable in host state"
        );
    }
}
