// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Curated facade crate for Portolan structured retrieval.
//!
//! A portolan was a mariner's chart: a practical map for navigation, routes,
//! landmarks, and making progress through a world too large to hold all at
//! once.
//!
//! Portolan helps hosts build command palettes, omniboxes, object pickers,
//! inspector search, and similar surfaces that need to search live things and
//! act on them.
//!
//! It sits between canonical host state, retrieval engines such as `leit_*`,
//! and interaction surfaces such as palettes, pickers, inspectors, or
//! automation systems. It is not a UI crate and not a retrieval kernel. It is
//! the layer that turns queries plus live host context into typed, actionable,
//! explainable candidates.
//!
//! In a typical flow, a host:
//! - defines its own subject type, such as commands or objects
//! - exposes one or more retrieval sources, such as a materialized
//!   [`crate::leit`] index, recents, or a visible workset
//! - routes one [`PortolanQuery`] plus one host-defined [`RetrievalContext`]
//! - receives [`PortolanHit`] values that carry score, provenance, evidence,
//!   and affordances
//! - resolves selected [`Affordance`] values back into host actions
//!
//! If you already have a search backend but still need to combine it with live
//! application state, verification, and surface-facing actions, this crate is
//! the layer above that backend.
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
//! - construct a [`PortolanQuery`]
//! - package one host-defined [`RetrievalContext`]
//! - run sources through a [`RetrievalRouter`]
//! - receive typed [`PortolanHit`] values with [`Evidence`] and [`Affordance`]
//!
//! For a fuller end-to-end example, see the workspace examples:
//! - `examples/command_palette`
//! - `examples/basic_routing`
//! - `examples/virtual_workset`
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
///
/// Reach for this module when you want the narrower `portolan_core` surface
/// rather than the curated top-level re-exports.
pub mod core {
    pub use portolan_core::*;
}

/// Observation and trace records.
///
/// This module exposes the `portolan_observe` crate for callers that want full
/// access to retrieval trace types such as [`RetrievalTrace`].
pub mod observe {
    pub use portolan_observe::*;
}

/// Query envelope types.
///
/// This module exposes the smaller `portolan_query` surface directly.
pub mod query {
    pub use portolan_query::*;
}

/// Staged routing, verification, and reconciliation.
///
/// This module exposes the routing layer directly when callers want the full
/// `portolan_route` API.
pub mod route {
    pub use portolan_route::*;
}

/// Source and sink seams.
///
/// This module exposes the lower-level source traits and sink helpers from
/// `portolan_source`.
pub mod source {
    pub use portolan_source::*;
}

#[cfg(feature = "schema")]
/// Subject projection and schema contracts.
///
/// Enable the `schema` feature to use these lower-level projection types.
pub mod schema {
    pub use portolan_schema::*;
}

#[cfg(feature = "ingest")]
/// Projection materialization helpers.
///
/// Enable the `ingest` feature to materialize projection catalogs into backend
/// indexes.
pub mod ingest {
    pub use portolan_ingest::*;
}

#[cfg(feature = "leit")]
/// Leit-backed retrieval adapters.
///
/// Enable the `leit` feature to use these materialized retrieval adapters.
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
