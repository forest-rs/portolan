// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Subject projection records and materialized field contracts for Portolan.
//!
//! This crate keeps the projection seam intentionally small:
//! - materialized fields are explicit `(field, text)` records
//! - subject identity remains host-defined
//! - optional affordances and metadata ride alongside the projection

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;

use leit_core::FieldId;
use portolan_core::{Affordance, StandardAffordance, SubjectRef};

/// One materialized text field emitted from a host projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedField {
    /// Field identifier understood by the retrieval backend.
    pub field: FieldId,
    /// Text materialized into that field.
    pub text: String,
}

impl MaterializedField {
    /// Create a new materialized field.
    pub fn new(field: FieldId, text: impl Into<String>) -> Self {
        Self {
            field,
            text: text.into(),
        }
    }
}

/// A host-defined subject projection ready for retrieval materialization.
#[derive(Clone, Debug, PartialEq)]
pub struct SubjectProjection<S: SubjectRef, A = StandardAffordance, M = ()> {
    /// Host-defined subject identity.
    pub subject: S,
    /// Materialized textual fields for retrieval backends such as Leit.
    pub materialized_fields: Vec<MaterializedField>,
    /// Affordances that should be attached when this subject becomes a hit.
    pub affordances: Vec<Affordance<A>>,
    /// Optional host-defined metadata carried with the projection.
    pub metadata: M,
}

impl<S: SubjectRef, A> SubjectProjection<S, A, ()> {
    /// Create a new projection with no metadata.
    pub fn new(subject: S, materialized_fields: Vec<MaterializedField>) -> Self {
        Self {
            subject,
            materialized_fields,
            affordances: Vec::new(),
            metadata: (),
        }
    }
}

impl<S: SubjectRef, A, M> SubjectProjection<S, A, M> {
    /// Replace the affordances attached to this projection.
    pub fn with_affordances(mut self, affordances: Vec<Affordance<A>>) -> Self {
        self.affordances = affordances;
        self
    }

    /// Replace the metadata carried with this projection.
    pub fn with_metadata<NewM>(self, metadata: NewM) -> SubjectProjection<S, A, NewM> {
        SubjectProjection {
            subject: self.subject,
            materialized_fields: self.materialized_fields,
            affordances: self.affordances,
            metadata,
        }
    }
}

/// Project host-owned values into Portolan subject projections.
pub trait ProjectSubject<Host, S: SubjectRef, A = StandardAffordance, M = ()> {
    /// Create a retrievable projection from one host value.
    fn project(&self, value: &Host) -> SubjectProjection<S, A, M>;
}
