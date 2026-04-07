// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retrieval trace records and observation helpers for Portolan.
//!
//! This crate intentionally stays generic over stage types so routing and
//! future execution layers can reuse the same trace vocabulary.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;

use portolan_core::RetrievalBudget;

/// Summary of one entered stage during retrieval execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRecord<Stage> {
    /// Stage that ran.
    pub stage: Stage,
    /// Number of sources invoked in this stage.
    pub sources_visited: u32,
    /// Number of hits emitted while this stage ran.
    pub hits_emitted: u32,
    /// Number of duplicate hits suppressed while this stage ran.
    pub duplicates_suppressed: u32,
}

/// Reason that routed retrieval stopped before exhausting the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason<Stage> {
    /// Routing stopped after a stage emitted at least the configured number of hits.
    StageHitLimitReached {
        /// Stage that caused the stop.
        stage: Stage,
        /// Hits emitted by that stage.
        hits_emitted: u32,
    },
    /// Routing stopped after total emitted hits reached the configured limit.
    TotalHitLimitReached {
        /// Stage during which the limit was reached.
        stage: Stage,
        /// Total emitted hits at stop time.
        hits_emitted: u32,
    },
}

/// One source visit during retrieval execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceVisit<Stage> {
    /// Stage in which the source ran.
    pub stage: Stage,
    /// Human-readable source label.
    pub source: String,
}

/// Trace of one retrieval pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetrievalTrace<Stage> {
    /// Raw query text.
    pub query: String,
    /// Budget used for this pass.
    pub budget: RetrievalBudget,
    /// Ordered list of source visits.
    pub visits: Vec<SourceVisit<Stage>>,
    /// Stage-level summaries in execution order.
    pub stages: Vec<StageRecord<Stage>>,
    /// Number of sources visited.
    pub sources_visited: u32,
    /// Number of stages entered.
    pub stages_visited: u32,
    /// Number of emitted hits.
    pub hits_emitted: u32,
    /// Number of duplicate hits suppressed before reaching the caller sink.
    pub duplicates_suppressed: u32,
    /// Reason retrieval stopped early, when applicable.
    pub stop_reason: Option<StopReason<Stage>>,
}

impl<Stage> RetrievalTrace<Stage> {
    /// Create an empty trace for one retrieval pass.
    pub fn new(query: impl Into<String>, budget: RetrievalBudget) -> Self {
        Self {
            query: query.into(),
            budget,
            visits: Vec::new(),
            stages: Vec::new(),
            sources_visited: 0,
            stages_visited: 0,
            hits_emitted: 0,
            duplicates_suppressed: 0,
            stop_reason: None,
        }
    }

    /// Record one source visit.
    pub fn record_visit(&mut self, stage: Stage, source: impl Into<String>) {
        self.sources_visited = self
            .sources_visited
            .checked_add(1)
            .expect("source visit count overflow");
        self.visits.push(SourceVisit {
            stage,
            source: source.into(),
        });
    }

    /// Record that one stage was entered.
    pub fn record_stage(
        &mut self,
        stage: Stage,
        sources_visited: u32,
        hits_emitted: u32,
        duplicates_suppressed: u32,
    ) {
        self.stages_visited = self
            .stages_visited
            .checked_add(1)
            .expect("stage visit count overflow");
        self.stages.push(StageRecord {
            stage,
            sources_visited,
            hits_emitted,
            duplicates_suppressed,
        });
        self.hits_emitted = self
            .hits_emitted
            .checked_add(hits_emitted)
            .expect("hit count overflow");
        self.duplicates_suppressed = self
            .duplicates_suppressed
            .checked_add(duplicates_suppressed)
            .expect("duplicate suppression count overflow");
    }

    /// Record that retrieval stopped early.
    pub fn record_stop_reason(&mut self, stop_reason: StopReason<Stage>) {
        self.stop_reason = Some(stop_reason);
    }
}
