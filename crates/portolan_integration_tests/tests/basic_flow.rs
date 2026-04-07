// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration test covering the minimal typed retrieval flow.

use leit_core::{FieldId, Score};
use portolan_core::{
    Affordance, Evidence, PortolanHit, RetrievalBudget, RetrievalContext, RetrievalOrigin,
    StandardAffordance,
};
use portolan_query::{ParsedQuery, PortolanQuery};
use portolan_source::{CandidateBuffer, CandidateSink, RetrievalSource};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DemoSubject(&'static str);

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
    let query = PortolanQuery::<(), ()>::text("open");
    let context = RetrievalContext::<(), (), (), ()>::default();
    let budget = RetrievalBudget::interactive_default();
    let source = DemoSource;
    let mut sink = CandidateBuffer::<DemoSubject>::new();

    source.retrieve_into(&query, &context, budget, &mut sink);

    assert_eq!(sink.len(), 1);
    assert_eq!(sink.as_slice()[0].subject, DemoSubject("demo.command.open"));
    assert_eq!(
        sink.as_slice()[0].origin,
        RetrievalOrigin::MaterializedIndex
    );
    assert_eq!(
        sink.as_slice()[0].affordances[0].action,
        StandardAffordance::Execute
    );
}
