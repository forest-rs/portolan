// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Core types and traits for Portolan structured retrieval.
//!
//! This crate provides the calm vocabulary used across the Portolan family:
//! - host-defined subject bounds
//! - typed retrieval hits
//! - evidence and provenance
//! - affordance descriptors and resolution seams
//! - retrieval context envelopes and budgets
//!
//! Most callers do not stay in this crate alone. They typically pair these
//! types with `PortolanQuery`, `RetrievalSource`, and `RetrievalRouter` to
//! build one retrieval pass.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::vec::Vec;
use core::fmt;
use core::hash::Hash;

pub use leit_core::{FieldId, Score};

/// Trait bound for host-defined retrievable subjects.
///
/// Portolan intentionally does not define one global subject enum.
/// Hosts provide their own subject universe and thread it through the retrieval
/// pipeline using this bound.
///
/// You usually do not implement this trait manually. Any host type that is
/// [`Clone`], [`Eq`], [`Hash`], and [`fmt::Debug`] automatically implements it
/// and can then appear in [`PortolanHit`] values, [`AffordanceResolver`]
/// implementations, and routing APIs.
pub trait SubjectRef: Clone + Eq + Hash + fmt::Debug {}

impl<T> SubjectRef for T where T: Clone + Eq + Hash + fmt::Debug {}

/// Built-in affordance kinds that Portolan understands semantically.
///
/// Hosts that do not need custom affordance payloads can use this enum as the
/// `A` parameter in [`Affordance`] and [`PortolanHit`]. Surface layers usually
/// turn these values into concrete actions through an
/// [`AffordanceResolver`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandardAffordance {
    /// Execute the target subject.
    Execute,
    /// Open the target subject.
    Open,
    /// Move focus to the target subject.
    Focus,
    /// Inspect the target subject.
    Inspect,
    /// Reveal the target subject in a larger structure.
    Reveal,
    /// Toggle a state associated with the target subject.
    Toggle,
    /// Preview the target subject.
    Preview,
    /// Refine the current query using the target subject.
    RefineQuery,
}

/// An affordance attached to a [`PortolanHit`].
///
/// Retrieval sources or enrichers attach affordances while assembling hits so
/// surfaces can later ask an [`AffordanceResolver`] what can be done with one
/// subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Affordance<A = StandardAffordance> {
    /// Host-defined affordance payload.
    pub action: A,
}

impl<A> Affordance<A> {
    /// Create a new affordance descriptor.
    pub const fn new(action: A) -> Self {
        Self { action }
    }
}

/// Evidence explaining why a [`PortolanHit`] matched.
///
/// Sources and enrichers produce evidence records while assembling hits.
/// Callers usually encounter this type by reading the [`PortolanHit::evidence`]
/// field rather than constructing it directly.
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence<K = ()> {
    /// Field associated with this evidence when known.
    pub field: Option<FieldId>,
    /// Score contribution attributed to this evidence.
    pub contribution: Score,
    /// Host-defined evidence classification.
    pub kind: K,
}

impl<K> Evidence<K> {
    /// Create a new evidence record with no associated field.
    pub const fn new(contribution: Score, kind: K) -> Self {
        Self {
            field: None,
            contribution,
            kind,
        }
    }

    /// Attach a field to this evidence record.
    pub const fn with_field(mut self, field: FieldId) -> Self {
        self.field = Some(field);
        self
    }
}

/// Describes where a [`PortolanHit`] originated.
///
/// Sources set this while constructing hits so later routing, diagnostics, and
/// surfaces can distinguish materialized retrieval from contextual or virtual
/// work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RetrievalOrigin {
    /// A materialized retrieval engine such as a Leit index.
    MaterializedIndex,
    /// A host-maintained contextual cache.
    ContextCache,
    /// A visible or already-loaded working set.
    VisibleWorkset,
    /// An on-demand or virtual scan.
    VirtualScan,
    /// A derived hit synthesized from other data.
    Derived,
}

/// A typed retrieval result.
///
/// This is the main value produced by Portolan retrieval. Retrieval source
/// implementations push hits into caller-provided sinks, and routed retrieval
/// eventually emits them to callers after optional verification and
/// reconciliation.
#[derive(Clone, Debug, PartialEq)]
pub struct PortolanHit<S: SubjectRef, A = StandardAffordance, E = ()> {
    /// Host-defined subject identity.
    pub subject: S,
    /// Final score assigned to this hit.
    pub score: Score,
    /// Evidence explaining the match.
    pub evidence: Vec<Evidence<E>>,
    /// Supported actions for the subject.
    pub affordances: Vec<Affordance<A>>,
    /// Origin of the hit.
    pub origin: RetrievalOrigin,
}

impl<S: SubjectRef, A, E> PortolanHit<S, A, E> {
    /// Create a hit with no evidence or affordances.
    ///
    /// This is the usual starting point inside retrieval sources before adding
    /// [`Evidence`] or [`Affordance`] values with the builder-style helpers.
    pub fn new(subject: S, score: Score, origin: RetrievalOrigin) -> Self {
        Self {
            subject,
            score,
            evidence: Vec::new(),
            affordances: Vec::new(),
            origin,
        }
    }

    /// Replace the affordances attached to this hit.
    pub fn with_affordances(mut self, affordances: Vec<Affordance<A>>) -> Self {
        self.affordances = affordances;
        self
    }

    /// Replace the evidence attached to this hit.
    pub fn with_evidence(mut self, evidence: Vec<Evidence<E>>) -> Self {
        self.evidence = evidence;
        self
    }

    /// Append one evidence record.
    pub fn push_evidence(&mut self, evidence: Evidence<E>) {
        self.evidence.push(evidence);
    }
}

/// Explicit retrieval work budget.
///
/// Callers pass one budget into a retrieval pass so sources and routers can
/// stay bounded. You typically construct this once and pass it through source
/// or router methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetrievalBudget {
    /// Candidate cap per source.
    pub max_candidates_per_source: u32,
    /// Cap on virtual expansions.
    pub max_virtual_expansions: u32,
    /// Cap on scanned nodes.
    pub max_nodes_scanned: u32,
    /// Wall-clock work budget in microseconds.
    pub max_time_us: u64,
}

impl RetrievalBudget {
    /// A conservative small default budget for interactive use.
    ///
    /// This is a good starting point for command palettes, omniboxes, and
    /// similar interactive surfaces.
    pub const fn interactive_default() -> Self {
        Self {
            max_candidates_per_source: 64,
            max_virtual_expansions: 16,
            max_nodes_scanned: 256,
            max_time_us: 5_000,
        }
    }
}

/// Explicit retrieval context envelope.
///
/// Portolan transports one host-defined context snapshot instead of hard-coding
/// multiple context lanes such as selection or focus. Hosts decide what
/// contextual state matters for one retrieval surface and package it into one
/// type.
///
/// Callers usually create one context per retrieval request and pass it into
/// source or router methods. Sources and verifiers then read from [`Self::host`]
/// when they need live host state.
///
/// ```
/// use portolan_core::RetrievalContext;
///
/// #[derive(Clone, Debug, PartialEq, Eq)]
/// struct HostState {
///     focus_id: &'static str,
/// }
///
/// let empty = RetrievalContext::<HostState>::default();
/// let context = RetrievalContext::with_host(HostState { focus_id: "camera.main" });
///
/// assert!(empty.host.is_none());
/// assert_eq!(context.host.as_ref().map(|state| state.focus_id), Some("camera.main"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetrievalContext<Host = ()> {
    /// Optional host-defined retrieval snapshot.
    pub host: Option<Host>,
}

impl<Host> RetrievalContext<Host> {
    /// Create a retrieval context from an optional host snapshot.
    ///
    /// Use this when the host snapshot is naturally optional at the call site.
    pub const fn new(host: Option<Host>) -> Self {
        Self { host }
    }

    /// Create a retrieval context that carries one host snapshot.
    ///
    /// This is the usual constructor when a retrieval surface always has
    /// contextual host state to thread through retrieval.
    pub fn with_host(host: Host) -> Self {
        Self { host: Some(host) }
    }
}

impl<Host> Default for RetrievalContext<Host> {
    fn default() -> Self {
        Self { host: None }
    }
}

/// Host-owned resolver for turning affordance descriptors into concrete actions.
///
/// Resolution is pure description lookup. Portolan does not execute the
/// resolved action.
///
/// Surfaces usually implement this trait after retrieval, once they are ready
/// to turn [`Affordance`] values on a [`PortolanHit`] into host-specific action
/// descriptors.
pub trait AffordanceResolver<S: SubjectRef, A> {
    /// Host-defined resolved action description.
    type Resolved;

    /// Resolve a hit affordance into a host action descriptor.
    fn resolve(&self, subject: &S, affordance: &Affordance<A>) -> Option<Self::Resolved>;
}
