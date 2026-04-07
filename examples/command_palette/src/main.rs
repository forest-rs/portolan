// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Example showing how to build a command-palette surface on top of Portolan.

use std::string::String;
use std::vec::Vec;

use leit_core::{FieldId, Score};
use leit_index::SearchScorer;
use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
use portolan_core::{
    Affordance, AffordanceResolver, Evidence, PortolanHit, RetrievalBudget, RetrievalContext,
    RetrievalOrigin, StandardAffordance,
};
use portolan_ingest::{FieldAlias, build_leit_index};
use portolan_leit::{CatalogHitEnricher, CatalogSubjectMapper, LeitSource, TextQueryLowerer};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_route::{RetrievalRouter, RoutePlan, RoutePolicy, RouteStage, StagedRetrievalSource};
use portolan_schema::{MaterializedField, ProjectSubject, ProjectionCatalog, SubjectProjection};
use portolan_source::{CandidateBuffer, CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PaletteSubject {
    Command(&'static str),
    Object(&'static str),
    Recent(&'static str),
}

#[derive(Clone, Debug)]
struct CommandRecord {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    category: &'static str,
}

#[derive(Clone, Debug)]
struct CommandMetadata {
    title: &'static str,
    subtitle: &'static str,
    category: &'static str,
}

#[derive(Clone, Debug)]
struct VisibleObject {
    id: &'static str,
    label: &'static str,
    subtitle: &'static str,
}

#[derive(Clone, Debug)]
struct RecentEntry {
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
}

#[derive(Clone, Debug)]
struct PaletteView {
    objects: Vec<VisibleObject>,
}

#[derive(Clone, Debug)]
struct PaletteRecent {
    entries: Vec<RecentEntry>,
}

type PaletteContext = RetrievalContext<(), (), PaletteView, PaletteRecent>;
type PaletteEvidence = &'static str;
type PaletteSourceRef<'a> = &'a dyn StagedRetrievalSource<
    PaletteSubject,
    (),
    (),
    (),
    (),
    PaletteView,
    PaletteRecent,
    StandardAffordance,
    PaletteEvidence,
>;
type LabeledPaletteSource<'a> = (&'a str, PaletteSourceRef<'a>);
type MaterializedPaletteSource<'a> = LeitSource<
    'a,
    CatalogSubjectMapper<'a, PaletteSubject, StandardAffordance, CommandMetadata>,
    TextQueryLowerer,
    CatalogHitEnricher<
        'a,
        PaletteSubject,
        StandardAffordance,
        CommandMetadata,
        fn(
            &SubjectProjection<PaletteSubject, StandardAffordance, CommandMetadata>,
            Score,
        ) -> Option<Evidence<PaletteEvidence>>,
    >,
>;

#[derive(Clone, Debug)]
struct PaletteAction {
    label: &'static str,
    target: String,
}

#[derive(Clone, Debug)]
struct PaletteItem {
    title: String,
    subtitle: String,
    actions: Vec<PaletteAction>,
    score: Score,
    origin: RetrievalOrigin,
    evidence: Vec<Evidence<PaletteEvidence>>,
}

#[derive(Clone, Debug)]
struct PaletteResponse {
    items: Vec<PaletteItem>,
    trace: portolan_observe::RetrievalTrace<RouteStage>,
}

struct PaletteResolver;

impl AffordanceResolver<PaletteSubject, StandardAffordance> for PaletteResolver {
    type Resolved = PaletteAction;

    fn resolve(
        &self,
        subject: &PaletteSubject,
        affordance: &Affordance<StandardAffordance>,
    ) -> Option<Self::Resolved> {
        match (subject, affordance.action) {
            (PaletteSubject::Command(id), StandardAffordance::Execute) => Some(PaletteAction {
                label: "Run",
                target: format!("execute {}", id),
            }),
            (PaletteSubject::Command(id), StandardAffordance::Open) => Some(PaletteAction {
                label: "Open",
                target: format!("open {}", id),
            }),
            (PaletteSubject::Object(id), StandardAffordance::Focus) => Some(PaletteAction {
                label: "Focus",
                target: format!("focus {}", id),
            }),
            (PaletteSubject::Object(id), StandardAffordance::Inspect) => Some(PaletteAction {
                label: "Inspect",
                target: format!("inspect {}", id),
            }),
            (PaletteSubject::Recent(id), StandardAffordance::Open) => Some(PaletteAction {
                label: "Reopen",
                target: format!("reopen {}", id),
            }),
            (PaletteSubject::Recent(id), StandardAffordance::RefineQuery) => Some(PaletteAction {
                label: "Use As Query",
                target: format!("refine query with {}", id),
            }),
            _ => None,
        }
    }
}

struct CommandProjector;

impl ProjectSubject<CommandRecord, PaletteSubject, StandardAffordance, CommandMetadata>
    for CommandProjector
{
    fn project(
        &self,
        value: &CommandRecord,
    ) -> SubjectProjection<PaletteSubject, StandardAffordance, CommandMetadata> {
        SubjectProjection::new(
            PaletteSubject::Command(value.id),
            vec![
                MaterializedField::new(FieldId::new(1), value.title),
                MaterializedField::new(FieldId::new(2), value.description),
                MaterializedField::new(FieldId::new(3), value.category),
            ],
        )
        .with_affordances(vec![Affordance::new(StandardAffordance::Execute)])
        .with_metadata(CommandMetadata {
            title: value.title,
            subtitle: value.description,
            category: value.category,
        })
    }
}

struct RecentSource;

impl
    RetrievalSource<
        PaletteSubject,
        (),
        (),
        (),
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for RecentSource
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &PaletteContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<PaletteSubject, StandardAffordance, PaletteEvidence>,
    ) {
        let text = match &query.parsed {
            ParsedQuery::Text { text }
            | ParsedQuery::Scoped { text, .. }
            | ParsedQuery::Structured { text, .. } => text.to_ascii_lowercase(),
        };
        let Some(recent) = &context.recent else {
            return;
        };
        let limit = usize::try_from(budget.max_candidates_per_source)
            .expect("candidate budget should fit in usize");
        let mut emitted = 0_usize;

        for entry in &recent.entries {
            if emitted >= limit {
                break;
            }

            let should_match = text.is_empty()
                || entry.title.to_ascii_lowercase().contains(&text)
                || entry.subtitle.to_ascii_lowercase().contains(&text);
            if !should_match {
                continue;
            }

            emitted += 1;
            out.push(PortolanHit {
                subject: PaletteSubject::Recent(entry.id),
                score: Score::new(0.45),
                evidence: vec![Evidence {
                    field: None,
                    contribution: Score::new(0.45),
                    kind: "recent_history",
                }],
                affordances: vec![
                    Affordance::new(StandardAffordance::Open),
                    Affordance::new(StandardAffordance::RefineQuery),
                ],
                origin: RetrievalOrigin::ContextCache,
            });
        }
    }
}

impl
    StagedRetrievalSource<
        PaletteSubject,
        (),
        (),
        (),
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for RecentSource
{
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

struct VisibleObjectSource;

impl
    RetrievalSource<
        PaletteSubject,
        (),
        (),
        (),
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for VisibleObjectSource
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &PaletteContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<PaletteSubject, StandardAffordance, PaletteEvidence>,
    ) {
        let text = match &query.parsed {
            ParsedQuery::Text { text }
            | ParsedQuery::Scoped { text, .. }
            | ParsedQuery::Structured { text, .. } => text.to_ascii_lowercase(),
        };
        let Some(view) = &context.visible_view else {
            return;
        };
        let limit = usize::try_from(budget.max_virtual_expansions)
            .expect("virtual budget should fit in usize");
        let mut emitted = 0_usize;

        for object in &view.objects {
            if emitted >= limit {
                break;
            }

            let should_match = object.label.to_ascii_lowercase().contains(&text)
                || object.subtitle.to_ascii_lowercase().contains(&text);
            if !should_match {
                continue;
            }

            emitted += 1;
            out.push(PortolanHit {
                subject: PaletteSubject::Object(object.id),
                score: Score::new(0.30),
                evidence: vec![Evidence {
                    field: None,
                    contribution: Score::new(0.30),
                    kind: "visible_object",
                }],
                affordances: vec![
                    Affordance::new(StandardAffordance::Focus),
                    Affordance::new(StandardAffordance::Inspect),
                ],
                origin: RetrievalOrigin::VirtualScan,
            });
        }
    }
}

impl
    StagedRetrievalSource<
        PaletteSubject,
        (),
        (),
        (),
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for VisibleObjectSource
{
    fn stage(&self) -> RouteStage {
        RouteStage::Virtual
    }
}

struct MaterializedSource<'a> {
    inner: MaterializedPaletteSource<'a>,
}

impl<'a> MaterializedSource<'a> {
    fn new(inner: MaterializedPaletteSource<'a>) -> Self {
        Self { inner }
    }
}

impl
    RetrievalSource<
        PaletteSubject,
        (),
        (),
        (),
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for MaterializedSource<'_>
{
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        context: &PaletteContext,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<PaletteSubject, StandardAffordance, PaletteEvidence>,
    ) {
        self.inner.retrieve_into(query, context, budget, out);
    }
}

impl
    StagedRetrievalSource<
        PaletteSubject,
        (),
        (),
        (),
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for MaterializedSource<'_>
{
    fn stage(&self) -> RouteStage {
        RouteStage::Materialized
    }
}

struct CommandPalette<'a> {
    router: RetrievalRouter,
    plan: RoutePlan,
    budget: RetrievalBudget,
    policy: RoutePolicy,
    sources: [LabeledPaletteSource<'a>; 3],
    catalog: &'a ProjectionCatalog<PaletteSubject, StandardAffordance, CommandMetadata>,
    resolver: PaletteResolver,
}

impl<'a> CommandPalette<'a> {
    fn new(
        sources: [LabeledPaletteSource<'a>; 3],
        catalog: &'a ProjectionCatalog<PaletteSubject, StandardAffordance, CommandMetadata>,
    ) -> Self {
        Self {
            router: RetrievalRouter::new(),
            plan: RoutePlan::standard(),
            budget: RetrievalBudget {
                max_candidates_per_source: 8,
                max_virtual_expansions: 4,
                max_nodes_scanned: 256,
                max_time_us: 5_000,
            },
            policy: RoutePolicy {
                stop_after_stage_hits: None,
                stop_after_total_hits: Some(3),
            },
            sources,
            catalog,
            resolver: PaletteResolver,
        }
    }

    fn search(&self, input: &str, context: &PaletteContext) -> PaletteResponse {
        let query = PortolanQuery::<(), ()>::text(input);
        let mut hits =
            CandidateBuffer::<PaletteSubject, StandardAffordance, PaletteEvidence>::new();
        let trace = self.router.retrieve_traced_with_policy(
            self.plan,
            self.policy,
            &self.sources,
            &query,
            context,
            self.budget,
            &mut hits,
        );
        let items = hits
            .into_hits()
            .into_iter()
            .map(|hit| self.render_item(hit, context))
            .collect();

        PaletteResponse { items, trace }
    }

    fn render_item(
        &self,
        hit: PortolanHit<PaletteSubject, StandardAffordance, PaletteEvidence>,
        context: &PaletteContext,
    ) -> PaletteItem {
        let (title, subtitle) = self.subject_text(&hit.subject, context);
        let actions = hit
            .affordances
            .iter()
            .filter_map(|affordance| self.resolver.resolve(&hit.subject, affordance))
            .collect();

        PaletteItem {
            title,
            subtitle,
            actions,
            score: hit.score,
            origin: hit.origin,
            evidence: hit.evidence,
        }
    }

    fn subject_text(&self, subject: &PaletteSubject, context: &PaletteContext) -> (String, String) {
        match subject {
            PaletteSubject::Command(_) => {
                let projection = self.command_projection(subject);
                (
                    projection.metadata.title.to_owned(),
                    format!(
                        "{} [{}]",
                        projection.metadata.subtitle, projection.metadata.category
                    ),
                )
            }
            PaletteSubject::Object(id) => {
                let object = context
                    .visible_view
                    .as_ref()
                    .and_then(|view| view.objects.iter().find(|object| object.id == *id))
                    .expect("palette object should exist in the visible view");
                (object.label.to_owned(), object.subtitle.to_owned())
            }
            PaletteSubject::Recent(id) => {
                let entry = context
                    .recent
                    .as_ref()
                    .and_then(|recent| recent.entries.iter().find(|entry| entry.id == *id))
                    .expect("recent palette entry should exist in history");
                (entry.title.to_owned(), entry.subtitle.to_owned())
            }
        }
    }

    fn command_projection(
        &self,
        subject: &PaletteSubject,
    ) -> &SubjectProjection<PaletteSubject, StandardAffordance, CommandMetadata> {
        let doc_id = self
            .catalog
            .doc_id_for_subject(subject)
            .expect("command subject should have a projection");
        self.catalog
            .projection(doc_id)
            .expect("catalog should retain the command projection")
    }
}

fn analyzers() -> FieldAnalyzers {
    let mut analyzers = FieldAnalyzers::new();
    let title = Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(1), title);
    let description =
        Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(2), description);
    let category =
        Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
    analyzers.set(FieldId::new(3), category);
    analyzers
}

fn projection_evidence(
    projection: &SubjectProjection<PaletteSubject, StandardAffordance, CommandMetadata>,
    score: Score,
) -> Option<Evidence<PaletteEvidence>> {
    Some(Evidence {
        field: projection
            .materialized_fields
            .first()
            .map(|field| field.field),
        contribution: score,
        kind: "command_projection",
    })
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

fn main() {
    let commands = [
        CommandRecord {
            id: "command.open_camera_panel",
            title: "Open Camera Panel",
            description: "Open camera controls in the inspector",
            category: "Navigation",
        },
        CommandRecord {
            id: "command.toggle_grid",
            title: "Toggle Grid",
            description: "Show or hide the viewport grid",
            category: "View",
        },
        CommandRecord {
            id: "command.capture_preview",
            title: "Capture Preview",
            description: "Render a preview from the active camera",
            category: "Render",
        },
    ];
    let projector = CommandProjector;
    let catalog = ProjectionCatalog::from_projections(
        commands.iter().map(|command| projector.project(command)),
    );
    let index = build_leit_index(
        &catalog,
        analyzers(),
        &[
            FieldAlias::new(FieldId::new(1), "title"),
            FieldAlias::new(FieldId::new(2), "description"),
            FieldAlias::new(FieldId::new(3), "category"),
        ],
    )
    .expect("palette commands should materialize into a Leit index");
    let materialized = MaterializedSource::new(
        LeitSource::new(
            &index,
            CatalogSubjectMapper::new(&catalog),
            SearchScorer::bm25(),
        )
        .with_enricher(
            CatalogHitEnricher::new(&catalog).with_evidence_builder(projection_evidence),
        ),
    );
    let recent = RecentSource;
    let visible_objects = VisibleObjectSource;
    let palette = CommandPalette::new(
        [
            ("palette.commands", &materialized),
            ("palette.recent", &recent),
            ("palette.visible_objects", &visible_objects),
        ],
        &catalog,
    );
    let context = PaletteContext {
        selection: None,
        focus: None,
        visible_view: Some(PaletteView {
            objects: vec![
                VisibleObject {
                    id: "object.camera.main",
                    label: "Main Camera",
                    subtitle: "Visible object in viewport",
                },
                VisibleObject {
                    id: "object.camera.preview",
                    label: "Preview Camera",
                    subtitle: "Off-screen preview camera",
                },
                VisibleObject {
                    id: "object.light.key",
                    label: "Key Light",
                    subtitle: "Visible object in viewport",
                },
            ],
        }),
        recent: Some(PaletteRecent {
            entries: vec![
                RecentEntry {
                    id: "recent.camera_panel",
                    title: "Open Camera Panel",
                    subtitle: "Recently used command",
                },
                RecentEntry {
                    id: "recent.capture_preview",
                    title: "Capture Preview",
                    subtitle: "Recently used command",
                },
            ],
        }),
    };

    println!("Portolan command palette example");
    println!();
    println!("1. Build a host-facing API on top of Portolan");
    println!("   command_palette.search(\"camera\", &context) -> PaletteResponse");
    println!(
        "   stages: {} -> {} -> {}",
        stage_label(RoutePlan::standard().stages()[0]),
        stage_label(RoutePlan::standard().stages()[1]),
        stage_label(RoutePlan::standard().stages()[2])
    );
    println!("   stop policy: stop after 3 total hits");
    println!();

    let response = palette.search("camera", &context);

    println!("2. Render the palette response");
    println!(
        "   visited {} stages across {} sources",
        response.trace.stages_visited, response.trace.sources_visited
    );
    println!("   trace:");
    for visit in &response.trace.visits {
        println!(
            "     - stage={} source={}",
            stage_label(visit.stage),
            visit.source
        );
    }
    println!("   stage summary:");
    for stage in &response.trace.stages {
        println!(
            "     - stage={} sources={} hits={}",
            stage_label(stage.stage),
            stage.sources_visited,
            stage.hits_emitted
        );
    }
    if let Some(stop_reason) = &response.trace.stop_reason {
        match stop_reason {
            portolan_observe::StopReason::StageHitLimitReached {
                stage,
                hits_emitted,
            } => println!(
                "   stopped early: stage {} reached {} hits",
                stage_label(*stage),
                hits_emitted
            ),
            portolan_observe::StopReason::TotalHitLimitReached {
                stage,
                hits_emitted,
            } => println!(
                "   stopped early: total hit limit reached in {} at {} hits",
                stage_label(*stage),
                hits_emitted
            ),
        }
    }

    for (index, item) in response.items.iter().enumerate() {
        println!(
            "   {}. {} | {} | score={:.3} | origin={}",
            index + 1,
            item.title,
            item.subtitle,
            item.score.as_f32(),
            origin_label(item.origin)
        );
        if !item.actions.is_empty() {
            println!("      actions:");
            for action in &item.actions {
                println!("        - {} => {}", action.label, action.target);
            }
        }
        for evidence in &item.evidence {
            println!(
                "      evidence: field={:?} contribution={:.3} kind={}",
                evidence.field.map(FieldId::as_u32),
                evidence.contribution.as_f32(),
                evidence.kind
            );
        }
    }
}
