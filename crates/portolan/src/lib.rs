// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Curated facade crate for Portolan structured retrieval.
//!
//! A portolan was a mariner's chart: a practical map for navigation, routes,
//! landmarks, and making progress through a world too large to hold all at
//! once.
//!
//! That is the intended shape of this crate.
//! Portolan sits between canonical host state, retrieval engines such as
//! `leit_*`, and interaction surfaces such as palettes, pickers, inspectors,
//! or automation systems. It is not a UI crate and not a retrieval kernel.
//! It is the layer that turns queries plus live host context into typed,
//! actionable, explainable candidates.
//!
//! This facade crate is the preferred way into the Portolan workspace when you
//! want the main retrieval path without importing many `portolan_*` crates
//! directly.
//!
//! It provides:
//! - top-level re-exports for the common retrieval workflow
//! - nested modules for lower-level or backend-specific layers
//! - explicit features for heavier integrations such as schema, ingest, and
//!   Leit-backed retrieval
//!
//! The intended top-level workflow is:
//! - construct a `PortolanQuery`
//! - package one host-defined `RetrievalContext`
//! - run sources through a `RetrievalRouter`
//! - receive typed `PortolanHit` values with evidence and affordances
//!
//! ```
//! use portolan::{PortolanQuery, RetrievalContext, RetrievalRouter, RoutePlan};
//!
//! let _query = PortolanQuery::<(), ()>::text("camera");
//! let _context = RetrievalContext::with_host("palette");
//! let _router = RetrievalRouter::new();
//! let _plan = RoutePlan::standard();
//! ```

#![no_std]

#[cfg(feature = "std")]
extern crate std;

/// Lower-level core vocabulary.
pub mod core {
    pub use portolan_core::*;
}

/// Observation and trace records.
pub mod observe {
    pub use portolan_observe::*;
}

/// Query envelope types.
pub mod query {
    pub use portolan_query::*;
}

/// Staged routing, verification, and reconciliation.
pub mod route {
    pub use portolan_route::*;
}

/// Source and sink seams.
pub mod source {
    pub use portolan_source::*;
}

#[cfg(feature = "schema")]
/// Subject projection and schema contracts.
pub mod schema {
    pub use portolan_schema::*;
}

#[cfg(feature = "ingest")]
/// Projection materialization helpers.
pub mod ingest {
    pub use portolan_ingest::*;
}

#[cfg(feature = "leit")]
/// Leit-backed retrieval adapters.
pub mod leit {
    pub use portolan_leit::*;
}

pub use portolan_core::{
    Affordance, AffordanceResolver, Evidence, FieldId, PortolanHit, RetrievalBudget,
    RetrievalContext, RetrievalOrigin, Score, StandardAffordance, SubjectRef,
};
pub use portolan_observe::{RetrievalTrace, StopReason};
pub use portolan_query::{ParsedQuery, PortolanQuery};
pub use portolan_route::{
    HitVerifier, HitVerifierExt, NoopHitVerifier, ReconciliationPolicy, RetrievalRouter, RoutePlan,
    RoutePolicy, RouteStage, RouteStats, StagedRetrievalSource, SubjectVerifier,
    VerificationOutcome, subject_verifier,
};
pub use portolan_source::{CandidateBuffer, CandidateSink, RetrievalSource};
