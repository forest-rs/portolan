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
    /// Number of sources visited.
    pub sources_visited: u32,
    /// Number of stages entered.
    pub stages_visited: u32,
}

impl<Stage> RetrievalTrace<Stage> {
    /// Create an empty trace for one retrieval pass.
    pub fn new(query: impl Into<String>, budget: RetrievalBudget) -> Self {
        Self {
            query: query.into(),
            budget,
            visits: Vec::new(),
            sources_visited: 0,
            stages_visited: 0,
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
    pub fn record_stage(&mut self) {
        self.stages_visited = self
            .stages_visited
            .checked_add(1)
            .expect("stage visit count overflow");
    }
}
