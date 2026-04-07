// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Projection-to-index materialization for Portolan.
//!
//! The first slice targets Leit's in-memory index builder.
//!
//! Callers usually build a [`ProjectionCatalog`] first, then pass it into
//! [`build_leit_index`] to obtain a materialized Leit [`InMemoryIndex`].

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use leit_core::FieldId;
use leit_index::{InMemoryIndex, InMemoryIndexBuilder, IndexError};
use leit_text::FieldAnalyzers;
use portolan_core::SubjectRef;
use portolan_schema::ProjectionCatalog;

/// Field alias registered for query planning in the materialized backend.
///
/// Callers construct these alongside analyzers when materializing a catalog
/// into a Leit index with [`build_leit_index`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldAlias<'a> {
    /// Retrieval field identifier.
    pub field: FieldId,
    /// User-facing alias for the field.
    pub alias: &'a str,
}

impl<'a> FieldAlias<'a> {
    /// Create a new field alias.
    pub const fn new(field: FieldId, alias: &'a str) -> Self {
        Self { field, alias }
    }
}

/// Build a Leit in-memory index from a [`ProjectionCatalog`].
///
/// This is the main first-slice materialization helper for Portolan. Callers
/// usually project host values into a catalog, prepare Leit analyzers and field
/// aliases, then call this function before constructing a Leit-backed retrieval
/// source.
pub fn build_leit_index<S: SubjectRef, A, M>(
    catalog: &ProjectionCatalog<S, A, M>,
    analyzers: FieldAnalyzers,
    field_aliases: &[FieldAlias<'_>],
) -> Result<InMemoryIndex, IndexError> {
    let mut builder = InMemoryIndexBuilder::new(analyzers);
    for alias in field_aliases {
        builder.register_field_alias(alias.field, alias.alias);
    }

    for (doc_id, projection) in catalog.iter() {
        let mut fields = alloc::vec::Vec::new();
        for field in &projection.materialized_fields {
            fields.push((field.field, field.text.as_str()));
        }
        builder.index_document(doc_id, &fields)?;
    }

    Ok(builder.build_index())
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::{FieldAlias, build_leit_index};
    use leit_core::FieldId;
    use leit_index::ExecutionWorkspace;
    use leit_text::{Analyzer, FieldAnalyzers, UnicodeNormalizer, WhitespaceTokenizer};
    use portolan_core::StandardAffordance;
    use portolan_schema::{MaterializedField, ProjectionCatalog, SubjectProjection};

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct DemoSubject(&'static str);

    fn analyzers() -> FieldAnalyzers {
        let mut analyzers = FieldAnalyzers::new();
        let analyzer =
            Analyzer::new(WhitespaceTokenizer::new()).with_normalizer(UnicodeNormalizer::new());
        analyzers.set(FieldId::new(1), analyzer);
        analyzers
    }

    #[test]
    fn builds_a_searchable_leit_index() {
        let catalog = ProjectionCatalog::from_projections([
            SubjectProjection::<DemoSubject, StandardAffordance>::new(
                DemoSubject("command.open"),
                vec![MaterializedField::new(FieldId::new(1), "Open Scene")],
            ),
            SubjectProjection::<DemoSubject, StandardAffordance>::new(
                DemoSubject("command.inspect"),
                vec![MaterializedField::new(FieldId::new(1), "Inspect Selection")],
            ),
        ])
        .expect("catalog should reject duplicate subjects");

        let index = build_leit_index(
            &catalog,
            analyzers(),
            &[FieldAlias::new(FieldId::new(1), "title")],
        )
        .expect("catalog should materialize");
        let mut workspace = ExecutionWorkspace::new();
        let hits = workspace
            .search(&index, "open", 5, leit_index::SearchScorer::bm25())
            .expect("search should succeed");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, 1);
    }
}
