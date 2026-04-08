// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Progressive live query session example for Portolan.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use portolan::live::{
    LiveCoordinator, LiveSession, LiveSource, SearchEvent, SearchEventBuffer, SnapshotLiveSource,
    SourceCapabilities, SourceEventSink, SourcePatchOp, SourceResult, SourceSearchEvent,
    SourceState, StagedLiveSource, StagedSnapshotSource, StatusUpdate,
};
use portolan::{
    Affordance, CandidateSink, PortolanHit, PortolanQuery, RetrievalBudget, RetrievalContext,
    RetrievalOrigin, RetrievalSource, RoutePlan, RouteStage, Score, StandardAffordance,
};

extern crate alloc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DemoSubject {
    Command(&'static str),
    Object(&'static str),
}

#[derive(Clone, Copy, Debug)]
struct RecentCommandSource;

impl RetrievalSource<DemoSubject> for RecentCommandSource {
    fn retrieve_into(
        &self,
        _query: &PortolanQuery,
        _context: &RetrievalContext,
        _budget: RetrievalBudget,
        out: &mut dyn CandidateSink<DemoSubject>,
    ) {
        out.push(
            PortolanHit::new(
                DemoSubject::Command("command.open_camera_panel"),
                Score::new(0.8),
                RetrievalOrigin::ContextCache,
            )
            .with_affordances(Vec::from([Affordance::new(StandardAffordance::Execute)])),
        );
    }
}

impl StagedSnapshotSource<DemoSubject> for RecentCommandSource {
    fn stage(&self) -> RouteStage {
        RouteStage::Contextual
    }
}

#[derive(Clone, Copy, Debug)]
struct RemoteObjectSource;

#[derive(Debug)]
struct RemoteObjectSession {
    session_id: portolan::QuerySessionId,
    step: u8,
}

impl LiveSession<DemoSubject, StandardAffordance, (), u8> for RemoteObjectSession {
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            streams_partial_results: true,
            revises_results: true,
            retracts_results: false,
            reports_progress: true,
            can_cancel: true,
        }
    }

    fn poll_events_into(
        &mut self,
        out: &mut dyn SourceEventSink<DemoSubject, StandardAffordance, (), u8>,
    ) -> portolan::PollOutcome {
        match self.step {
            0 => {
                out.push(SourceSearchEvent::Begin {
                    session_id: self.session_id,
                    revision: portolan::Revision::new(0),
                    capabilities: self.capabilities(),
                });
                self.step = 1;
                portolan::PollOutcome {
                    emitted_events: 1,
                    terminal: false,
                }
            }
            1 => {
                out.push(SourceSearchEvent::Progress {
                    session_id: self.session_id,
                    revision: portolan::Revision::new(1),
                    progress: 40_u8,
                });
                out.push(SourceSearchEvent::ApplyPatch {
                    session_id: self.session_id,
                    revision: portolan::Revision::new(2),
                    ops: vec![SourcePatchOp::Insert {
                        result: SourceResult::new(
                            portolan::LocalResultId::new(7),
                            PortolanHit::new(
                                DemoSubject::Object("object.camera.main"),
                                Score::new(0.45),
                                RetrievalOrigin::VirtualScan,
                            )
                            .with_affordances(Vec::from([
                                Affordance::new(StandardAffordance::Focus),
                                Affordance::new(StandardAffordance::Inspect),
                            ])),
                        ),
                    }],
                });
                self.step = 2;
                portolan::PollOutcome {
                    emitted_events: 2,
                    terminal: false,
                }
            }
            2 => {
                out.push(SourceSearchEvent::Progress {
                    session_id: self.session_id,
                    revision: portolan::Revision::new(3),
                    progress: 100_u8,
                });
                out.push(SourceSearchEvent::ApplyPatch {
                    session_id: self.session_id,
                    revision: portolan::Revision::new(4),
                    ops: vec![SourcePatchOp::Replace {
                        result: SourceResult::new(
                            portolan::LocalResultId::new(7),
                            PortolanHit::new(
                                DemoSubject::Object("object.camera.main"),
                                Score::new(0.93),
                                RetrievalOrigin::VirtualScan,
                            )
                            .with_affordances(Vec::from([
                                Affordance::new(StandardAffordance::Focus),
                                Affordance::new(StandardAffordance::Inspect),
                            ])),
                        ),
                    }],
                });
                out.push(SourceSearchEvent::StatusChanged {
                    session_id: self.session_id,
                    revision: portolan::Revision::new(5),
                    status: StatusUpdate::new(SourceState::Complete),
                });
                self.step = 3;
                portolan::PollOutcome {
                    emitted_events: 3,
                    terminal: true,
                }
            }
            _ => portolan::PollOutcome {
                emitted_events: 0,
                terminal: true,
            },
        }
    }

    fn cancel(&mut self) {
        self.step = 3;
    }
}

impl LiveSource<DemoSubject, (), (), (), StandardAffordance, (), u8> for RemoteObjectSource {
    fn begin_session(
        &self,
        session_id: portolan::QuerySessionId,
        _query: PortolanQuery,
        _context: RetrievalContext,
        _budget: RetrievalBudget,
    ) -> Box<dyn LiveSession<DemoSubject, StandardAffordance, (), u8> + 'static> {
        Box::new(RemoteObjectSession {
            session_id,
            step: 0,
        })
    }
}

impl StagedLiveSource<DemoSubject, (), (), (), StandardAffordance, (), u8> for RemoteObjectSource {
    fn stage(&self) -> RouteStage {
        RouteStage::Virtual
    }
}

fn main() {
    let recent = SnapshotLiveSource::new(RecentCommandSource);
    let remote = RemoteObjectSource;
    let sources = [
        (
            "recent-commands",
            &recent as &dyn StagedLiveSource<DemoSubject, (), (), (), StandardAffordance, (), u8>,
        ),
        (
            "remote-objects",
            &remote as &dyn StagedLiveSource<DemoSubject, (), (), (), StandardAffordance, (), u8>,
        ),
    ];
    let coordinator = LiveCoordinator::new();
    let mut session = coordinator.begin_session(
        RoutePlan::standard(),
        &sources,
        PortolanQuery::<(), ()>::text("camera"),
        RetrievalContext::default(),
        RetrievalBudget::interactive_default(),
    );
    let mut events = SearchEventBuffer::new();

    println!("Portolan live session example\n");
    println!("query: camera");
    println!("sources: recent snapshot + remote progressive source\n");

    let mut round = 1_u32;
    loop {
        let poll = session.poll_events_into(&mut events);
        println!("poll round {round}");
        for event in events.take() {
            print_event(&event);
        }
        println!();

        if poll.terminal {
            break;
        }
        round += 1;
    }
}

fn print_event(event: &SearchEvent<&str, DemoSubject, StandardAffordance, (), u8>) {
    match event {
        SearchEvent::SessionStarted { session_id } => {
            println!("  session started: {}", session_id.get());
        }
        SearchEvent::SourceStarted {
            source,
            stage,
            capabilities,
            ..
        } => {
            println!(
                "  source started: {source} ({}) partials={} revises={} progress={}",
                stage_label(*stage),
                capabilities.streams_partial_results,
                capabilities.revises_results,
                capabilities.reports_progress
            );
        }
        SearchEvent::Progress {
            source, progress, ..
        } => {
            println!("  progress: {source} -> {progress}%");
        }
        SearchEvent::ApplyPatch {
            source, stage, ops, ..
        } => {
            println!("  patch: {source} ({})", stage_label(*stage));
            for op in ops {
                match op {
                    portolan::SessionPatchOp::Insert { result } => {
                        println!(
                            "    insert {:?} score={:.2} id={}:{}",
                            result.hit.subject,
                            result.hit.score.as_f32(),
                            result.id.source_slot.get(),
                            result.id.local.get()
                        );
                    }
                    portolan::SessionPatchOp::Replace { result } => {
                        println!(
                            "    replace {:?} score={:.2} id={}:{}",
                            result.hit.subject,
                            result.hit.score.as_f32(),
                            result.id.source_slot.get(),
                            result.id.local.get()
                        );
                    }
                    portolan::SessionPatchOp::Remove { result_id } => {
                        println!(
                            "    remove id={}:{}",
                            result_id.source_slot.get(),
                            result_id.local.get()
                        );
                    }
                    portolan::SessionPatchOp::MoveBefore { result_id, anchor } => {
                        println!(
                            "    move id={}:{} before {:?}",
                            result_id.source_slot.get(),
                            result_id.local.get(),
                            anchor
                                .as_ref()
                                .map(|anchor| (anchor.source_slot.get(), anchor.local.get(),))
                        );
                    }
                }
            }
        }
        SearchEvent::StatusChanged { source, status, .. } => {
            println!("  status: {source} -> {:?}", status.state);
        }
        SearchEvent::SessionFinished { session_id, .. } => {
            println!("  session finished: {}", session_id.get());
        }
    }
}

const fn stage_label(stage: RouteStage) -> &'static str {
    match stage {
        RouteStage::Materialized => "materialized",
        RouteStage::Contextual => "contextual",
        RouteStage::Virtual => "virtual",
    }
}
