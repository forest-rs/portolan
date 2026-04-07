// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Source and sink traits for Portolan retrieval.
//!
//! The first slice is synchronous and explicit:
//! sources receive a query, context, and budget, then push candidates into a
//! caller-provided sink. Later crates may add routing, staging, and deferred
//! execution on top of these seams.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::vec::Vec;

use portolan_core::{PortolanHit, RetrievalBudget, RetrievalContext, SubjectRef};
use portolan_query::PortolanQuery;

/// Sink for retrieval candidates.
pub trait CandidateSink<S: SubjectRef, A = portolan_core::StandardAffordance, E = ()> {
    /// Push one candidate into the sink.
    fn push(&mut self, hit: PortolanHit<S, A, E>);
}

/// A simple growable candidate sink backed by a `Vec`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CandidateBuffer<S: SubjectRef, A = portolan_core::StandardAffordance, E = ()> {
    hits: Vec<PortolanHit<S, A, E>>,
}

impl<S: SubjectRef, A, E> CandidateBuffer<S, A, E> {
    /// Create an empty candidate buffer.
    pub fn new() -> Self {
        Self { hits: Vec::new() }
    }

    /// Number of retained hits.
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// Whether the buffer contains no hits.
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// Borrow retained hits.
    pub fn as_slice(&self) -> &[PortolanHit<S, A, E>] {
        &self.hits
    }

    /// Consume the buffer and return the retained hits.
    pub fn into_hits(self) -> Vec<PortolanHit<S, A, E>> {
        self.hits
    }
}

impl<S: SubjectRef, A, E> CandidateSink<S, A, E> for CandidateBuffer<S, A, E> {
    fn push(&mut self, hit: PortolanHit<S, A, E>) {
        self.hits.push(hit);
    }
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
