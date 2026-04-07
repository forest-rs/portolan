// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Staged multi-source routing for Portolan retrieval.
//!
//! The first routing slice stays deliberately small:
//! - sources declare a stage
//! - a route plan picks stage order
//! - the router executes sources stage by stage into a caller sink

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use portolan_core::{RetrievalBudget, RetrievalContext, SubjectRef};
use portolan_observe::RetrievalTrace;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RouteStats {
    /// Number of sources invoked.
    pub sources_visited: u32,
    /// Number of stages entered.
    pub stages_visited: u32,
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
        Self::route(plan, sources, query, context, budget, out, None)
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
        let source_refs: alloc::vec::Vec<_> = sources.iter().map(|(_, source)| *source).collect();
        let mut trace = RetrievalTrace::new(query.raw.clone(), budget);
        let _ = Self::route(
            plan,
            &source_refs,
            query,
            context,
            budget,
            out,
            Some((&mut trace, sources)),
        );
        trace
    }

    fn route<S: SubjectRef, Scope, Filter, Selection, Focus, View, Recent, A, E>(
        plan: RoutePlan,
        sources: SourceList<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
        mut trace: MaybeTraceState<'_, S, Scope, Filter, Selection, Focus, View, Recent, A, E>,
    ) -> RouteStats {
        let mut stats = RouteStats::default();

        for stage in plan.stages() {
            let mut visited_this_stage = false;
            for (source_index, source) in sources.iter().enumerate() {
                if source.stage() != *stage {
                    continue;
                }

                visited_this_stage = true;
                stats.sources_visited += 1;
                if let Some((trace, labeled_sources)) = &mut trace {
                    trace.record_visit(*stage, labeled_sources[source_index].0);
                }
                source.retrieve_into(query, context, budget, out);
            }

            if visited_this_stage {
                stats.stages_visited += 1;
                if let Some((trace, _)) = &mut trace {
                    trace.record_stage();
                }
            }
        }

        stats
    }
}
