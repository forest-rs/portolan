// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Staged multi-source routing for Portolan retrieval.
//!
//! The first routing slice stays deliberately small:
//! - sources declare a stage
//! - a route plan picks stage order
//! - the router executes sources stage by stage into a caller sink
//! - optional stop, dedup, and verification policies keep the work budgeted and explicit

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use portolan_core::{PortolanHit, RetrievalBudget, RetrievalContext, SubjectRef};
use portolan_observe::{RetrievalTrace, StopReason};
use portolan_query::PortolanQuery;
use portolan_source::{CandidateSink, RetrievalSource};

/// Retrieval stage used for route planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RouteStage {
    /// Materialized indexed retrieval.
    Materialized,
    /// Contextual cached or visible-workset retrieval.
    Contextual,
    /// On-demand or virtual expansion.
    Virtual,
}

/// Statistics collected during staged routing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteStats {
    /// Number of sources invoked.
    pub sources_visited: u32,
    /// Number of stages entered.
    pub stages_visited: u32,
    /// Number of emitted hits.
    pub hits_emitted: u32,
    /// Number of duplicate hits suppressed before reaching the caller sink.
    pub duplicates_suppressed: u32,
    /// Number of hits rejected by verification before reaching the caller sink.
    pub hits_rejected: u32,
    /// Reason retrieval stopped early, when applicable.
    pub stop_reason: Option<StopReason<RouteStage>>,
}

/// Outcome of verifying one routed hit before it reaches the caller sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationOutcome {
    /// Retain the hit and continue routing.
    Retain,
    /// Reject the hit and keep it out of the caller sink.
    Reject,
}

/// Host-owned verifier for finalizing routed hits against canonical state.
pub trait HitVerifier<
    S: SubjectRef,
    Selection = (),
    Focus = (),
    View = (),
    Recent = (),
    A = portolan_core::StandardAffordance,
    E = (),
>
{
    /// Verify one hit before it reaches the caller sink.
    fn verify_hit(
        &self,
        hit: &mut PortolanHit<S, A, E>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
    ) -> VerificationOutcome;
}

impl<S, Selection, Focus, View, Recent, A, E, F>
    HitVerifier<S, Selection, Focus, View, Recent, A, E> for F
where
    S: SubjectRef,
    F: Fn(
        &mut PortolanHit<S, A, E>,
        &RetrievalContext<Selection, Focus, View, Recent>,
    ) -> VerificationOutcome,
{
    fn verify_hit(
        &self,
        hit: &mut PortolanHit<S, A, E>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
    ) -> VerificationOutcome {
        self(hit, context)
    }
}

/// Verifier that retains every hit unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopHitVerifier;

impl<S, Selection, Focus, View, Recent, A, E> HitVerifier<S, Selection, Focus, View, Recent, A, E>
    for NoopHitVerifier
where
    S: SubjectRef,
{
    fn verify_hit(
        &self,
        _hit: &mut PortolanHit<S, A, E>,
        _context: &RetrievalContext<Selection, Focus, View, Recent>,
    ) -> VerificationOutcome {
        VerificationOutcome::Retain
    }
}

/// Policy controlling how routed retrieval handles repeated subjects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DuplicatePolicy {
    /// Retain every hit, even when multiple sources emit the same subject.
    #[default]
    RetainAll,
    /// Keep the first retained hit for each subject and suppress later duplicates.
    KeepFirstBySubject,
}

/// Explicit policy controlling when routing may stop before exhausting the plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoutePolicy {
    /// Stop after any stage emits at least this many hits.
    pub stop_after_stage_hits: Option<u32>,
    /// Stop after total emitted hits reach at least this many hits.
    pub stop_after_total_hits: Option<u32>,
    /// Policy for handling repeated subjects emitted by multiple sources.
    pub duplicate_policy: DuplicatePolicy,
}

impl RoutePolicy {
    /// Policy that always exhausts the route plan.
    pub const fn exhaustive() -> Self {
        Self {
            stop_after_stage_hits: None,
            stop_after_total_hits: None,
            duplicate_policy: DuplicatePolicy::RetainAll,
        }
    }
}

/// Object-safe retrieval source with an explicit route stage.
pub trait StagedRetrievalSource<
    S: SubjectRef,
    Scope = (),
    Filter = (),
    Selection = (),
    Focus = (),
    View = (),
    Recent = (),
    A = portolan_core::StandardAffordance,
    E = (),
>: RetrievalSource<S, Scope, Filter, Selection, Focus, View, Recent, A, E>
{
    /// Stage in which this source should run.
    fn stage(&self) -> RouteStage;
}

type SourceRef<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E> =
    &'a dyn StagedRetrievalSource<S, Scope, Filter, Selection, Focus, View, Recent, A, E>;

type LabeledSource<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E> = (
    &'a str,
    SourceRef<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
);

type SourceList<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E> =
    &'a [SourceRef<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E>];

type LabeledSourceList<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E> =
    &'a [LabeledSource<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E>];

type TraceState<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E> = (
    &'a mut RetrievalTrace<RouteStage>,
    LabeledSourceList<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
);

type MaybeTraceState<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E> =
    Option<TraceState<'a, S, Scope, Filter, Selection, Focus, View, Recent, A, E>>;

/// Stage order for one retrieval pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoutePlan {
    stages: [RouteStage; 3],
}

impl RoutePlan {
    /// Plan that prefers cheap materialized retrieval before more expensive work.
    pub const fn standard() -> Self {
        Self {
            stages: [
                RouteStage::Materialized,
                RouteStage::Contextual,
                RouteStage::Virtual,
            ],
        }
    }

    /// Access the stage order.
    pub const fn stages(&self) -> &[RouteStage; 3] {
        &self.stages
    }
}

impl Default for RoutePlan {
    fn default() -> Self {
        Self::standard()
    }
}

/// Router that executes staged retrieval sources in order.
#[derive(Clone, Copy, Debug, Default)]
pub struct RetrievalRouter;

impl RetrievalRouter {
    /// Create a new retrieval router.
    pub const fn new() -> Self {
        Self
    }

    /// Execute all matching sources in route-plan order.
    pub fn retrieve_into<S: SubjectRef, Scope, Filter, Selection, Focus, View, Recent, A, E>(
        &self,
        plan: RoutePlan,
        sources: SourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RouteStats {
        self.retrieve_verified_with_policy(
            plan,
            RoutePolicy::exhaustive(),
            sources,
            query,
            context,
            budget,
            &NoopHitVerifier,
            out,
        )
    }

    /// Execute all matching sources in route-plan order with an explicit stop policy.
    pub fn retrieve_with_policy<
        S: SubjectRef,
        Scope,
        Filter,
        Selection,
        Focus,
        View,
        Recent,
        A,
        E,
    >(
        &self,
        plan: RoutePlan,
        policy: RoutePolicy,
        sources: SourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RouteStats {
        self.retrieve_verified_with_policy(
            plan,
            policy,
            sources,
            query,
            context,
            budget,
            &NoopHitVerifier,
            out,
        )
    }

    /// Execute all matching sources in route-plan order and verify hits before emitting them.
    pub fn retrieve_verified_into<
        S: SubjectRef,
        Scope,
        Filter,
        Selection,
        Focus,
        View,
        Recent,
        A,
        E,
        Verifier,
    >(
        &self,
        plan: RoutePlan,
        sources: SourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        verifier: &Verifier,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RouteStats
    where
        Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
    {
        self.retrieve_verified_with_policy(
            plan,
            RoutePolicy::exhaustive(),
            sources,
            query,
            context,
            budget,
            verifier,
            out,
        )
    }

    /// Execute all matching sources in route-plan order with an explicit stop policy and verifier.
    pub fn retrieve_verified_with_policy<
        S: SubjectRef,
        Scope,
        Filter,
        Selection,
        Focus,
        View,
        Recent,
        A,
        E,
        Verifier,
    >(
        &self,
        plan: RoutePlan,
        policy: RoutePolicy,
        sources: SourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        verifier: &Verifier,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RouteStats
    where
        Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
    {
        Self::route(
            plan, policy, sources, query, context, budget, verifier, out, None,
        )
    }

    /// Execute labeled sources and capture a retrieval trace.
    pub fn retrieve_traced<S: SubjectRef, Scope, Filter, Selection, Focus, View, Recent, A, E>(
        &self,
        plan: RoutePlan,
        sources: LabeledSourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RetrievalTrace<RouteStage> {
        self.retrieve_traced_verified_with_policy(
            plan,
            RoutePolicy::exhaustive(),
            sources,
            query,
            context,
            budget,
            &NoopHitVerifier,
            out,
        )
    }

    /// Execute labeled sources with an explicit stop policy and capture a retrieval trace.
    pub fn retrieve_traced_with_policy<
        S: SubjectRef,
        Scope,
        Filter,
        Selection,
        Focus,
        View,
        Recent,
        A,
        E,
    >(
        &self,
        plan: RoutePlan,
        policy: RoutePolicy,
        sources: LabeledSourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RetrievalTrace<RouteStage> {
        self.retrieve_traced_verified_with_policy(
            plan,
            policy,
            sources,
            query,
            context,
            budget,
            &NoopHitVerifier,
            out,
        )
    }

    /// Execute labeled sources, verify hits, and capture a retrieval trace.
    pub fn retrieve_traced_verified<
        S: SubjectRef,
        Scope,
        Filter,
        Selection,
        Focus,
        View,
        Recent,
        A,
        E,
        Verifier,
    >(
        &self,
        plan: RoutePlan,
        sources: LabeledSourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        verifier: &Verifier,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RetrievalTrace<RouteStage>
    where
        Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
    {
        self.retrieve_traced_verified_with_policy(
            plan,
            RoutePolicy::exhaustive(),
            sources,
            query,
            context,
            budget,
            verifier,
            out,
        )
    }

    /// Execute labeled sources with an explicit stop policy and verifier, and capture a retrieval trace.
    pub fn retrieve_traced_verified_with_policy<
        S: SubjectRef,
        Scope,
        Filter,
        Selection,
        Focus,
        View,
        Recent,
        A,
        E,
        Verifier,
    >(
        &self,
        plan: RoutePlan,
        policy: RoutePolicy,
        sources: LabeledSourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        verifier: &Verifier,
        out: &mut dyn CandidateSink<S, A, E>,
    ) -> RetrievalTrace<RouteStage>
    where
        Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
    {
        let source_refs: alloc::vec::Vec<_> = sources.iter().map(|(_, source)| *source).collect();
        let mut trace = RetrievalTrace::new(query.raw.clone(), budget);
        let _ = Self::route(
            plan,
            policy,
            &source_refs,
            query,
            context,
            budget,
            verifier,
            out,
            Some((&mut trace, sources)),
        );
        trace
    }

    fn route<S: SubjectRef, Scope, Filter, Selection, Focus, View, Recent, A, E, Verifier>(
        plan: RoutePlan,
        policy: RoutePolicy,
        sources: SourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        verifier: &Verifier,
        out: &mut dyn CandidateSink<S, A, E>,
        mut trace: MaybeTraceState<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
    ) -> RouteStats
    where
        Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
    {
        let mut stats = RouteStats::default();
        let mut seen_subjects = alloc::vec::Vec::new();

        for stage in plan.stages() {
            let mut visited_this_stage = false;
            let mut stage_sources_visited = 0_u32;
            let stage_hit_base = stats.hits_emitted;
            let stage_duplicate_base = stats.duplicates_suppressed;
            let stage_rejected_base = stats.hits_rejected;
            for (source_index, source) in sources.iter().enumerate() {
                if source.stage() != *stage {
                    continue;
                }

                visited_this_stage = true;
                stats.sources_visited += 1;
                stage_sources_visited += 1;
                if let Some((trace, labeled_sources)) = &mut trace {
                    trace.record_visit(*stage, labeled_sources[source_index].0);
                }
                let mut routing_sink = RoutingSink::new(
                    out,
                    context,
                    verifier,
                    policy.duplicate_policy,
                    &mut seen_subjects,
                );
                source.retrieve_into(query, context, budget, &mut routing_sink);
                stats.hits_emitted = stats
                    .hits_emitted
                    .checked_add(routing_sink.hits_emitted())
                    .expect("route hit count overflow");
                stats.duplicates_suppressed = stats
                    .duplicates_suppressed
                    .checked_add(routing_sink.duplicates_suppressed())
                    .expect("route duplicate suppression count overflow");
                stats.hits_rejected = stats
                    .hits_rejected
                    .checked_add(routing_sink.hits_rejected())
                    .expect("route verification rejection count overflow");
            }

            if visited_this_stage {
                let stage_hits_emitted = stats.hits_emitted - stage_hit_base;
                let stage_duplicates_suppressed =
                    stats.duplicates_suppressed - stage_duplicate_base;
                let stage_hits_rejected = stats.hits_rejected - stage_rejected_base;
                stats.stages_visited = stats
                    .stages_visited
                    .checked_add(1)
                    .expect("route stage count overflow");
                if let Some((trace, _)) = &mut trace {
                    trace.record_stage(
                        *stage,
                        stage_sources_visited,
                        stage_hits_emitted,
                        stage_duplicates_suppressed,
                        stage_hits_rejected,
                    );
                }

                if let Some(stop_reason) =
                    stop_reason_for_stage(policy, *stage, stage_hits_emitted, stats.hits_emitted)
                {
                    stats.stop_reason = Some(stop_reason.clone());
                    if let Some((trace, _)) = &mut trace {
                        trace.record_stop_reason(stop_reason);
                    }
                    break;
                }
            }
        }

        stats
    }
}

fn stop_reason_for_stage(
    policy: RoutePolicy,
    stage: RouteStage,
    stage_hits_emitted: u32,
    total_hits_emitted: u32,
) -> Option<StopReason<RouteStage>> {
    if let Some(limit) = policy.stop_after_stage_hits
        && stage_hits_emitted >= limit
    {
        return Some(StopReason::StageHitLimitReached {
            stage,
            hits_emitted: stage_hits_emitted,
        });
    }

    if let Some(limit) = policy.stop_after_total_hits
        && total_hits_emitted >= limit
    {
        return Some(StopReason::TotalHitLimitReached {
            stage,
            hits_emitted: total_hits_emitted,
        });
    }

    None
}

struct RoutingSink<'a, S: SubjectRef, Selection, Focus, View, Recent, A, E, Verifier> {
    inner: &'a mut dyn CandidateSink<S, A, E>,
    context: &'a RetrievalContext<Selection, Focus, View, Recent>,
    verifier: &'a Verifier,
    duplicate_policy: DuplicatePolicy,
    seen_subjects: &'a mut alloc::vec::Vec<S>,
    hits_emitted: u32,
    duplicates_suppressed: u32,
    hits_rejected: u32,
}

impl<'a, S: SubjectRef, Selection, Focus, View, Recent, A, E, Verifier>
    RoutingSink<'a, S, Selection, Focus, View, Recent, A, E, Verifier>
where
    Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
{
    fn new(
        inner: &'a mut dyn CandidateSink<S, A, E>,
        context: &'a RetrievalContext<Selection, Focus, View, Recent>,
        verifier: &'a Verifier,
        duplicate_policy: DuplicatePolicy,
        seen_subjects: &'a mut alloc::vec::Vec<S>,
    ) -> Self {
        Self {
            inner,
            context,
            verifier,
            duplicate_policy,
            seen_subjects,
            hits_emitted: 0,
            duplicates_suppressed: 0,
            hits_rejected: 0,
        }
    }

    fn hits_emitted(&self) -> u32 {
        self.hits_emitted
    }

    fn duplicates_suppressed(&self) -> u32 {
        self.duplicates_suppressed
    }

    fn hits_rejected(&self) -> u32 {
        self.hits_rejected
    }
}

impl<S: SubjectRef, Selection, Focus, View, Recent, A, E, Verifier> CandidateSink<S, A, E>
    for RoutingSink<'_, S, Selection, Focus, View, Recent, A, E, Verifier>
where
    Verifier: HitVerifier<S, Selection, Focus, View, Recent, A, E>,
{
    fn push(&mut self, mut hit: PortolanHit<S, A, E>) {
        if matches!(
            self.verifier.verify_hit(&mut hit, self.context),
            VerificationOutcome::Reject
        ) {
            self.hits_rejected = self
                .hits_rejected
                .checked_add(1)
                .expect("counting sink verification rejection overflow");
            return;
        }

        if matches!(self.duplicate_policy, DuplicatePolicy::KeepFirstBySubject)
            && self.seen_subjects.iter().any(|seen| seen == &hit.subject)
        {
            self.duplicates_suppressed = self
                .duplicates_suppressed
                .checked_add(1)
                .expect("counting sink duplicate suppression overflow");
            return;
        }

        if matches!(self.duplicate_policy, DuplicatePolicy::KeepFirstBySubject) {
            self.seen_subjects.push(hit.subject.clone());
        }

        self.hits_emitted = self
            .hits_emitted
            .checked_add(1)
            .expect("counting sink hit count overflow");
        self.inner.push(hit);
    }
}
