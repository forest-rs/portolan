// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Subject projection records and materialized field contracts for Portolan.
//!
//! This crate keeps the projection seam intentionally small:
//! - materialized fields are explicit `(field, text)` records
//! - subject identity remains host-defined
//! - optional affordances and metadata ride alongside the projection
//!
//! Hosts usually enter this crate by implementing [`ProjectSubject`] for one
//! host-owned record type, then collecting the results in a
//! [`ProjectionCatalog`] for materialization or lookup.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use hashbrown::HashMap;

use leit_core::FieldId;
use portolan_core::{Affordance, StandardAffordance, SubjectRef};

/// One materialized text field emitted from a host projection.
///
/// [`SubjectProjection`] values hold zero or more materialized fields.
/// Materialization code consumes them when building retrieval indexes.
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
///
/// Hosts usually obtain this value from a [`ProjectSubject`] implementation.
/// It packages one subject's materialized text, optional [`Affordance`] values,
/// and any host metadata that should travel with the projection.
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
    ///
    /// This is the usual starting point inside a [`ProjectSubject`]
    /// implementation before attaching affordances or metadata.
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
    ///
    /// Use this when the eventual retrieval hits for this subject should carry
    /// predeclared actions.
    pub fn with_affordances(mut self, affordances: Vec<Affordance<A>>) -> Self {
        self.affordances = affordances;
        self
    }

    /// Replace the metadata carried with this projection.
    ///
    /// Use this when the host wants to keep additional structured data
    /// alongside the materialized fields.
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
///
/// Hosts implement this trait at the boundary where application data becomes
/// retrieval-ready [`SubjectProjection`] values.
pub trait ProjectSubject<Host, S: SubjectRef, A = StandardAffordance, M = ()> {
    /// Create a retrievable projection from one host value.
    fn project(&self, value: &Host) -> SubjectProjection<S, A, M>;
}

/// Error returned when a [`ProjectionCatalog`] cannot preserve its identity invariants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionCatalogError<S: SubjectRef> {
    /// The subject is already present in the catalog.
    DuplicateSubject {
        /// Subject that would have been inserted twice.
        subject: S,
    },
}

impl<S: SubjectRef> fmt::Display for ProjectionCatalogError<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSubject { subject } => {
                write!(f, "projection catalog already contains subject {subject:?}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl<S: SubjectRef> std::error::Error for ProjectionCatalogError<S> {}

/// Stable catalog of projections keyed by retrieval document ID.
///
/// This is the main container type for projected subjects. Callers usually
/// build one catalog from an iterator of [`SubjectProjection`] values and then
/// hand it to materialization or adapter code.
#[derive(Clone, Debug)]
pub struct ProjectionCatalog<S: SubjectRef, A = StandardAffordance, M = ()> {
    projections: Vec<SubjectProjection<S, A, M>>,
    doc_ids_by_subject: HashMap<S, u32>,
}

impl<S: SubjectRef, A, M> Default for ProjectionCatalog<S, A, M> {
    fn default() -> Self {
        Self {
            projections: Vec::new(),
            doc_ids_by_subject: HashMap::new(),
        }
    }
}

impl<S: SubjectRef, A, M> ProjectionCatalog<S, A, M> {
    /// Create an empty projection catalog.
    ///
    /// Use this when projections will be inserted incrementally.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one projection and return its stable document ID.
    ///
    /// The returned document ID is stable for the lifetime of this catalog and
    /// is the identifier backend adapters should use for this projection.
    pub fn insert(
        &mut self,
        projection: SubjectProjection<S, A, M>,
    ) -> Result<u32, ProjectionCatalogError<S>> {
        if self.doc_ids_by_subject.contains_key(&projection.subject) {
            return Err(ProjectionCatalogError::DuplicateSubject {
                subject: projection.subject,
            });
        }

        let doc_id =
            u32::try_from(self.projections.len() + 1).expect("projection count should fit in u32");
        self.doc_ids_by_subject
            .insert(projection.subject.clone(), doc_id);
        self.projections.push(projection);
        Ok(doc_id)
    }

    /// Build a catalog from an iterator of projections.
    ///
    /// This is the usual constructor when the host can project all subjects up
    /// front.
    pub fn from_projections(
        projections: impl IntoIterator<Item = SubjectProjection<S, A, M>>,
    ) -> Result<Self, ProjectionCatalogError<S>> {
        let mut catalog = Self::new();
        for projection in projections {
            let _ = catalog.insert(projection)?;
        }
        Ok(catalog)
    }

    /// Number of stored projections.
    pub fn len(&self) -> usize {
        self.projections.len()
    }

    /// Whether the catalog contains no projections.
    pub fn is_empty(&self) -> bool {
        self.projections.is_empty()
    }

    /// Borrow the projection for one document ID.
    ///
    /// Callers typically use this after a backend returns a document ID and the
    /// host wants to recover the projected data.
    pub fn projection(&self, doc_id: u32) -> Option<&SubjectProjection<S, A, M>> {
        self.projections.get(doc_id.checked_sub(1)? as usize)
    }

    /// Borrow the subject for one document ID.
    ///
    /// This is a lighter-weight lookup than [`Self::projection`] when only the
    /// stable subject identity is needed.
    pub fn subject(&self, doc_id: u32) -> Option<&S> {
        Some(&self.projection(doc_id)?.subject)
    }

    /// Look up the document ID for one subject.
    ///
    /// Adapters and tests use this to move from host subject identity back to
    /// the stable document ID space.
    pub fn doc_id_for_subject(&self, subject: &S) -> Option<u32> {
        self.doc_ids_by_subject.get(subject).copied()
    }

    /// Iterate over projections with stable document IDs.
    ///
    /// Materialization code uses this to stream the catalog into a retrieval
    /// backend.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &SubjectProjection<S, A, M>)> {
        self.projections
            .iter()
            .enumerate()
            .map(|(index, projection)| {
                (
                    u32::try_from(index + 1).expect("projection index should fit in u32"),
                    projection,
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::{ProjectionCatalog, ProjectionCatalogError, SubjectProjection};
    use leit_core::FieldId;
    use portolan_core::StandardAffordance;

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct DemoSubject(&'static str);

    #[test]
    fn assigns_stable_doc_ids_and_supports_reverse_lookup() {
        let mut catalog = ProjectionCatalog::<DemoSubject, StandardAffordance>::new();
        let first = catalog
            .insert(SubjectProjection::new(
                DemoSubject("command.open"),
                vec![super::MaterializedField::new(FieldId::new(1), "Open")],
            ))
            .expect("first subject should insert");
        let second = catalog
            .insert(SubjectProjection::new(
                DemoSubject("command.inspect"),
                vec![super::MaterializedField::new(FieldId::new(1), "Inspect")],
            ))
            .expect("second subject should insert");

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(catalog.subject(1), Some(&DemoSubject("command.open")));
        assert_eq!(
            catalog.doc_id_for_subject(&DemoSubject("command.inspect")),
            Some(2)
        );
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn rejects_duplicate_subjects() {
        let mut catalog = ProjectionCatalog::<DemoSubject, StandardAffordance>::new();
        let _ = catalog
            .insert(SubjectProjection::new(
                DemoSubject("command.open"),
                vec![super::MaterializedField::new(FieldId::new(1), "Open")],
            ))
            .expect("first subject should insert");

        let duplicate = catalog.insert(SubjectProjection::new(
            DemoSubject("command.open"),
            vec![super::MaterializedField::new(FieldId::new(1), "Open Again")],
        ));

        assert_eq!(
            duplicate,
            Err(ProjectionCatalogError::DuplicateSubject {
                subject: DemoSubject("command.open")
            })
        );
        assert_eq!(catalog.len(), 1);
        assert_eq!(
            catalog.doc_id_for_subject(&DemoSubject("command.open")),
            Some(1)
        );
    }
}
