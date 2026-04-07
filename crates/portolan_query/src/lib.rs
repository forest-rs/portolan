// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small query types for Portolan.
//!
//! The common query model stays deliberately narrow:
//! - raw text
//! - optional scope
//! - optional filters
//! - a parsed envelope that hosts may lower further

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;

/// A host-extensible structured query envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortolanQuery<Scope = (), Filter = ()> {
    /// User-provided raw text.
    pub raw: String,
    /// Parsed query envelope.
    pub parsed: ParsedQuery<Scope, Filter>,
}

impl<Scope, Filter> PortolanQuery<Scope, Filter> {
    /// Create a new query from raw text and a parsed form.
    pub fn new(raw: impl Into<String>, parsed: ParsedQuery<Scope, Filter>) -> Self {
        Self {
            raw: raw.into(),
            parsed,
        }
    }
}

/// A minimal parsed query shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedQuery<Scope = (), Filter = ()> {
    /// Unstructured text query.
    Text {
        /// Free text to match against retrieval sources.
        text: String,
    },
    /// Text query with an explicit scope.
    Scoped {
        /// Host-defined scope token.
        scope: Scope,
        /// Free text constrained by the scope.
        text: String,
    },
    /// Text query with host-defined filters.
    Structured {
        /// Host-defined structured filters.
        filters: Vec<Filter>,
        /// Free text combined with the filters.
        text: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{ParsedQuery, PortolanQuery};

    #[test]
    fn preserves_raw_and_parsed_forms() {
        let query = PortolanQuery::new(
            "open scene",
            ParsedQuery::<(), ()>::Text {
                text: "open scene".into(),
            },
        );

        assert_eq!(query.raw, "open scene");
        assert_eq!(
            query.parsed,
            ParsedQuery::Text {
                text: "open scene".into(),
            }
        );
    }
}
