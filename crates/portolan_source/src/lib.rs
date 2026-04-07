// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Source and sink traits for Portolan retrieval.
//!
//! The first slice is synchronous and explicit:
//! sources receive a query, context, and budget, then push candidates into a
//! caller-provided sink. Later crates may add routing, staging, and deferred
//! execution on top of these seams.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

use portolan_core::{PortolanHit, RetrievalBudget, RetrievalContext, SubjectRef};
use portolan_query::PortolanQuery;

/// Sink for retrieval candidates.
pub trait CandidateSink<S: SubjectRef, A = portolan_core::StandardAffordance, E = ()> {
    /// Push one candidate into the sink.
    fn push(&mut self, hit: PortolanHit<S, A, E>);
}

/// A retrieval source in the first synchronous Portolan slice.
pub trait RetrievalSource<
    S: SubjectRef,
    Scope = (),
    Filter = (),
    Selection = (),
    Focus = (),
    View = (),
    Recent = (),
    A = portolan_core::StandardAffordance,
    E = (),
>
{
    /// Retrieve candidates into the caller-provided sink.
    fn retrieve_into(
        &self,
        query: &PortolanQuery<Scope, Filter>,
        context: &RetrievalContext<Selection, Focus, View, Recent>,
        budget: RetrievalBudget,
        out: &mut dyn CandidateSink<S, A, E>,
    );
}
