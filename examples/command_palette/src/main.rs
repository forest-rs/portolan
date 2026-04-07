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
use portolan_leit::{CatalogHitEnricher, CatalogSubjectMapper, LeitSource};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_route::{
    HitVerifier, ReconciliationPolicy, RetrievalRouter, RoutePlan, RoutePolicy, RouteStage,
    StagedRetrievalSource, VerificationOutcome,
};
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

#[derive(Clone, Debug)]
struct PaletteTruth {
    object_ids: Vec<&'static str>,
    recent_ids: Vec<&'static str>,
}

type PaletteContext = RetrievalContext<PaletteTruth, (), PaletteView, PaletteRecent>;
type PaletteEvidence = &'static str;
type PaletteSourceRef<'a> = &'a dyn StagedRetrievalSource<
    PaletteSubject,
    (),
    (),
    PaletteTruth,
    (),
    PaletteView,
    PaletteRecent,
    StandardAffordance,
    PaletteEvidence,
>;
type LabeledPaletteSource<'a> = (&'a str, PaletteSourceRef<'a>);
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

struct PaletteVerifier<'a> {
    catalog: &'a ProjectionCatalog<PaletteSubject, StandardAffordance, CommandMetadata>,
}

impl<'a> PaletteVerifier<'a> {
    const fn new(
        catalog: &'a ProjectionCatalog<PaletteSubject, StandardAffordance, CommandMetadata>,
    ) -> Self {
        Self { catalog }
    }
}

impl
    HitVerifier<
        PaletteSubject,
        PaletteTruth,
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for PaletteVerifier<'_>
{
    fn verify_hit(
        &self,
        hit: &mut PortolanHit<PaletteSubject, StandardAffordance, PaletteEvidence>,
        context: &PaletteContext,
    ) -> VerificationOutcome {
        match &hit.subject {
            PaletteSubject::Command(_) => {
                if self.catalog.doc_id_for_subject(&hit.subject).is_some() {
                    VerificationOutcome::Retain
                } else {
                    VerificationOutcome::Reject
                }
            }
            PaletteSubject::Object(id) => {
                context
                    .selection
                    .as_ref()
                    .map_or(VerificationOutcome::Reject, |truth| {
                        if truth.object_ids.iter().any(|candidate| candidate == id) {
                            VerificationOutcome::Retain
                        } else {
                            VerificationOutcome::Reject
                        }
                    })
            }
            PaletteSubject::Recent(id) => {
                context
                    .selection
                    .as_ref()
                    .map_or(VerificationOutcome::Reject, |truth| {
                        if truth.recent_ids.iter().any(|candidate| candidate == id) {
                            VerificationOutcome::Retain
                        } else {
                            VerificationOutcome::Reject
                        }
                    })
            }
        }
    }
}

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
        PaletteTruth,
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
            out.push(
                PortolanHit::new(
                    PaletteSubject::Recent(entry.id),
                    Score::new(0.45),
                    RetrievalOrigin::ContextCache,
                )
                .with_evidence(vec![Evidence::new(Score::new(0.45), "recent_history")])
                .with_affordances(vec![
                    Affordance::new(StandardAffordance::Open),
                    Affordance::new(StandardAffordance::RefineQuery),
                ]),
            );
        }
    }
}

impl
    StagedRetrievalSource<
        PaletteSubject,
        (),
        (),
        PaletteTruth,
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
        PaletteTruth,
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
            out.push(
                PortolanHit::new(
                    PaletteSubject::Object(object.id),
                    Score::new(0.30),
                    RetrievalOrigin::VirtualScan,
                )
                .with_evidence(vec![Evidence::new(Score::new(0.30), "visible_object")])
                .with_affordances(vec![
                    Affordance::new(StandardAffordance::Focus),
                    Affordance::new(StandardAffordance::Inspect),
                ]),
            );
        }
    }
}

impl
    StagedRetrievalSource<
        PaletteSubject,
        (),
        (),
        PaletteTruth,
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

struct MaterializedSource<Inner> {
    inner: Inner,
}

impl<Inner> MaterializedSource<Inner> {
    fn new(inner: Inner) -> Self {
        Self { inner }
    }
}

impl<Inner>
    RetrievalSource<
        PaletteSubject,
        (),
        (),
        PaletteTruth,
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for MaterializedSource<Inner>
where
    Inner: RetrievalSource<
            PaletteSubject,
            (),
            (),
            PaletteTruth,
            (),
            PaletteView,
            PaletteRecent,
            StandardAffordance,
            PaletteEvidence,
        >,
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

impl<Inner>
    StagedRetrievalSource<
        PaletteSubject,
        (),
        (),
        PaletteTruth,
        (),
        PaletteView,
        PaletteRecent,
        StandardAffordance,
        PaletteEvidence,
    > for MaterializedSource<Inner>
where
    Inner: RetrievalSource<
            PaletteSubject,
            (),
            (),
            PaletteTruth,
            (),
            PaletteView,
            PaletteRecent,
            StandardAffordance,
            PaletteEvidence,
        >,
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
    verifier: PaletteVerifier<'a>,
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
                reconciliation_policy: ReconciliationPolicy::KeepFirstBySubject,
            },
            sources,
            catalog,
            verifier: PaletteVerifier::new(catalog),
            resolver: PaletteResolver,
        }
    }

    fn search(&self, input: &str, context: &PaletteContext) -> PaletteResponse {
        let query = PortolanQuery::<(), ()>::text(input);
        let mut hits =
            CandidateBuffer::<PaletteSubject, StandardAffordance, PaletteEvidence>::new();
        let trace = self.router.retrieve_traced_verified_with_policy(
            self.plan,
            self.policy,
            &self.sources,
            &query,
            context,
            self.budget,
            &self.verifier,
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
            PaletteSubject::Command(_) => self.command_projection(subject).map_or_else(
                || self.stale_subject_text(subject),
                |projection| {
                    (
                        projection.metadata.title.to_owned(),
                        format!(
                            "{} [{}]",
                            projection.metadata.subtitle, projection.metadata.category
                        ),
                    )
                },
            ),
            PaletteSubject::Object(id) => context
                .visible_view
                .as_ref()
                .and_then(|view| view.objects.iter().find(|object| object.id == *id))
                .map(|object| (object.label.to_owned(), object.subtitle.to_owned()))
                .unwrap_or_else(|| self.stale_subject_text(subject)),
            PaletteSubject::Recent(id) => context
                .recent
                .as_ref()
                .and_then(|recent| recent.entries.iter().find(|entry| entry.id == *id))
                .map(|entry| (entry.title.to_owned(), entry.subtitle.to_owned()))
                .unwrap_or_else(|| self.stale_subject_text(subject)),
        }
    }

    fn command_projection(
        &self,
        subject: &PaletteSubject,
    ) -> Option<&SubjectProjection<PaletteSubject, StandardAffordance, CommandMetadata>> {
        let doc_id = self.catalog.doc_id_for_subject(subject)?;
        self.catalog.projection(doc_id)
    }

    fn stale_subject_text(&self, subject: &PaletteSubject) -> (String, String) {
        let (kind, id) = match subject {
            PaletteSubject::Command(id) => ("Command", *id),
            PaletteSubject::Object(id) => ("Object", *id),
            PaletteSubject::Recent(id) => ("Recent", *id),
        };

        (
            format!("[stale] {kind}"),
            format!("subject {id} is no longer available in host state"),
        )
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
    )
    .expect("example should not contain duplicate subjects");
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
            CatalogHitEnricher::new(&catalog).with_first_field_evidence("command_projection"),
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
        selection: Some(PaletteTruth {
            object_ids: vec![
                "object.camera.main",
                "object.camera.preview",
                "object.light.key",
            ],
            recent_ids: vec!["recent.camera_panel"],
        }),
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
                RecentEntry {
                    id: "recent.stale_camera_cache",
                    title: "Open Camera Panel",
                    subtitle: "Cached stale suggestion",
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
    println!("   reconciliation policy: keep first hit per subject");
    println!("   verification: reject hits that are no longer present in host truth");
    println!();

    let response = palette.search("camera", &context);

    println!("2. Render the palette response");
    println!("   host truth:");
    println!("     - valid recent ids: recent.camera_panel");
    println!("     - stale cached recent ids will be rejected before rendering");
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
            "     - stage={} sources={} hits={} duplicates_suppressed={} hits_replaced={} hits_rejected={}",
            stage_label(stage.stage),
            stage.sources_visited,
            stage.hits_emitted,
            stage.duplicates_suppressed,
            stage.hits_replaced,
            stage.hits_rejected
        );
    }
    if response.trace.duplicates_suppressed > 0 {
        println!(
            "   duplicates suppressed: {}",
            response.trace.duplicates_suppressed
        );
    }
    if response.trace.hits_rejected > 0 {
        println!(
            "   hits rejected by verification: {}",
            response.trace.hits_rejected
        );
    }
    if response.trace.hits_replaced > 0 {
        println!(
            "   hits replaced during reconciliation: {}",
            response.trace.hits_replaced
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_palette() -> CommandPalette<'static> {
        let catalog = Box::leak(Box::new(ProjectionCatalog::new()));
        let sources: [LabeledPaletteSource<'static>; 3] = [
            ("palette.commands", &NoopSource),
            ("palette.recent", &NoopSource),
            ("palette.visible_objects", &NoopSource),
        ];

        CommandPalette::new(sources, catalog)
    }

    struct NoopSource;

    impl
        RetrievalSource<
            PaletteSubject,
            (),
            (),
            PaletteTruth,
            (),
            PaletteView,
            PaletteRecent,
            StandardAffordance,
            PaletteEvidence,
        > for NoopSource
    {
        fn retrieve_into(
            &self,
            _query: &PortolanQuery,
            _context: &PaletteContext,
            _budget: RetrievalBudget,
            _out: &mut dyn CandidateSink<PaletteSubject, StandardAffordance, PaletteEvidence>,
        ) {
        }
    }

    impl
        StagedRetrievalSource<
            PaletteSubject,
            (),
            (),
            PaletteTruth,
            (),
            PaletteView,
            PaletteRecent,
            StandardAffordance,
            PaletteEvidence,
        > for NoopSource
    {
        fn stage(&self) -> RouteStage {
            RouteStage::Materialized
        }
    }

    #[test]
    fn renders_stale_subjects_without_panicking() {
        let palette = empty_palette();
        let context = PaletteContext {
            selection: Some(PaletteTruth {
                object_ids: Vec::new(),
                recent_ids: Vec::new(),
            }),
            focus: None,
            visible_view: Some(PaletteView {
                objects: Vec::new(),
            }),
            recent: Some(PaletteRecent {
                entries: Vec::new(),
            }),
        };

        let command = palette.subject_text(&PaletteSubject::Command("command.missing"), &context);
        let object = palette.subject_text(&PaletteSubject::Object("object.missing"), &context);
        let recent = palette.subject_text(&PaletteSubject::Recent("recent.missing"), &context);

        assert_eq!(command.0, "[stale] Command");
        assert_eq!(
            command.1,
            "subject command.missing is no longer available in host state"
        );
        assert_eq!(object.0, "[stale] Object");
        assert_eq!(
            object.1,
            "subject object.missing is no longer available in host state"
        );
        assert_eq!(recent.0, "[stale] Recent");
        assert_eq!(
            recent.1,
            "subject recent.missing is no longer available in host state"
        );
    }

    #[test]
    fn search_rejects_stale_cached_recent_hits() {
        let commands = [CommandRecord {
            id: "command.open_camera_panel",
            title: "Open Camera Panel",
            description: "Open camera controls in the inspector",
            category: "Navigation",
        }];
        let projector = CommandProjector;
        let catalog = Box::leak(Box::new(
            ProjectionCatalog::from_projections(
                commands.iter().map(|command| projector.project(command)),
            )
            .expect("test commands should not duplicate subjects"),
        ));
        let index = build_leit_index(
            catalog,
            analyzers(),
            &[
                FieldAlias::new(FieldId::new(1), "title"),
                FieldAlias::new(FieldId::new(2), "description"),
                FieldAlias::new(FieldId::new(3), "category"),
            ],
        )
        .expect("test commands should materialize");
        let materialized = Box::leak(Box::new(MaterializedSource::new(
            LeitSource::new(
                &index,
                CatalogSubjectMapper::new(catalog),
                SearchScorer::bm25(),
            )
            .with_enricher(
                CatalogHitEnricher::new(catalog).with_first_field_evidence("command_projection"),
            ),
        )));
        let recent = Box::leak(Box::new(RecentSource));
        let visible_objects = Box::leak(Box::new(VisibleObjectSource));
        let palette = CommandPalette::new(
            [
                ("palette.commands", materialized),
                ("palette.recent", recent),
                ("palette.visible_objects", visible_objects),
            ],
            catalog,
        );
        let context = PaletteContext {
            selection: Some(PaletteTruth {
                object_ids: vec![],
                recent_ids: vec!["recent.camera_panel"],
            }),
            focus: None,
            visible_view: Some(PaletteView {
                objects: Vec::new(),
            }),
            recent: Some(PaletteRecent {
                entries: vec![
                    RecentEntry {
                        id: "recent.camera_panel",
                        title: "Open Camera Panel",
                        subtitle: "Recently used command",
                    },
                    RecentEntry {
                        id: "recent.stale_camera_cache",
                        title: "Open Camera Panel",
                        subtitle: "Cached stale suggestion",
                    },
                ],
            }),
        };

        let response = palette.search("camera", &context);

        assert_eq!(response.trace.hits_rejected, 1);
        assert!(
            response
                .items
                .iter()
                .all(|item| item.title != "[stale] Recent")
        );
        assert_eq!(response.items.len(), 2);
    }
}
