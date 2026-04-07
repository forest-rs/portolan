// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration test covering the minimal typed retrieval flow.

use leit_core::{FieldId, Score};
use portolan_core::{
    Affordance, Evidence, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin,
    StandardAffordance,
};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_source::{CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DemoSubject(&'static str);

#[derive(Default)]
struct VecSink(Vec<PortolanHit<DemoSubject>>);

impl CandidateSink<DemoSubject> for VecSink {
    fn push(&mut self, hit: PortolanHit<DemoSubject>) {
        self.0.push(hit);
    }
}

struct DemoSource;

impl RetrievalSource<DemoSubject> for DemoSource {
    fn retrieve_into(
        &self,
        query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        out.push(PortolanHit {
            subject: DemoSubject("demo.command.open"),
            score: Score::new(1.0),
            evidence: vec![Evidence {
                field: Some(FieldId::new(1)),
                contribution: Score::new(1.0),
                kind: (),
            }],
            affordances: vec![Affordance::new(StandardAffordance::Execute)],
            origin: match &query.parsed {
                ParsedQuery::Text { .. } => RetrievalOrigin::MaterializedIndex,
                ParsedQuery::Scoped { .. } | ParsedQuery::Structured { .. } => {
                    RetrievalOrigin::Derived
                }
            },
        });
    }
}

#[test]
fn retrieves_typed_actionable_candidates() {
    let query = PortolanQuery::new(
        "open",
        ParsedQuery::<(), ()>::Text {
            text: "open".into(),
        },
    );
    let context = RetrievalContext::<(), (), (), ()>::default();
    let budget = RetrievalBudget::interactive_default();
    let source = DemoSource;
    let mut sink = VecSink::default();

    source.retrieve_into(&query, &context, budget, &mut sink);

    assert_eq!(sink.0.len(), 1);
    assert_eq!(sink.0[0].subject, DemoSubject("demo.command.open"));
    assert_eq!(sink.0[0].origin, RetrievalOrigin::MaterializedIndex);
    assert_eq!(sink.0[0].affordances[0].action, StandardAffordance::Execute);
}
