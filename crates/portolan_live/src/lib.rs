// Copyright 2026 the Portolan Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Live query sessions and incremental retrieval updates for Portolan.
//!
//! This crate adds a session-based observation model above the calm one-shot
//! [`portolan_source::RetrievalSource`] seam.
//!
//! Use it when retrieval needs to:
//! - reveal results progressively
//! - revise or retract earlier results
//! - report status or progress over time
//! - survive long enough for a surface to poll or drain several rounds of
//!   updates
//!
//! The main types are:
//! - [`LiveSource`] and [`LiveSession`] for source-local live retrieval
//! - [`SourceSearchEvent`] and [`SourcePatchOp`] for source-local updates
//! - [`LiveCoordinator`] and [`LiveCoordinatorSession`] for multi-source
//!   normalization
//! - [`SearchEvent`] and [`SessionPatchOp`] for the coordinated event stream
//! - [`SnapshotLiveSource`] for lifting cloneable `'static` one-shot sources
//!   into live sessions
//!
//! The live layer does not require async runtimes or channels. Callers open one
//! session, poll or drain events explicitly, and cancel the session when it is
//! no longer relevant.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::Cell;
use core::fmt;

use portolan_core::{
    PortolanHit, RetrievalBudget, RetrievalContext, StandardAffordance, SubjectRef,
};
use portolan_query::PortolanQuery;
use portolan_route::{RoutePlan, RouteStage};
use portolan_source::{CandidateBuffer, RetrievalSource};

/// Stable identity for one live query session.
///
/// [`LiveCoordinator`] assigns these when it begins a session, and all
/// coordinated [`SearchEvent`] values carry the session id so callers can drop
/// stale updates cleanly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QuerySessionId(u64);

impl QuerySessionId {
    /// Create one session id from a raw integer.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Revision counter for live updates within one session.
///
/// Coordinators and sessions increment revisions as they emit events so
/// callers can reject out-of-date updates that arrive after newer ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Revision(u64);

impl Revision {
    /// Create one revision from a raw integer.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable source-local result id within one live source session.
///
/// Sources assign these ids when they first reveal one result. Later patch
/// events refer back to the same ids when they revise, move, or remove that
/// same subject.
///
/// A [`SourcePatchOp::Replace`] may change score, evidence, affordances, or
/// other hit details, but it must preserve the subject already associated with
/// the id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalResultId(u64);

impl LocalResultId {
    /// Create one local result id from a raw integer.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable source slot assigned by the coordinator within one session.
///
/// Session-global result ids combine one source slot with one
/// [`LocalResultId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceSlot(u16);

impl SourceSlot {
    /// Create one source slot from a raw integer.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Raw numeric value.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Stable result id in the coordinator-level event stream.
///
/// This combines one source slot plus one source-local result id. That keeps
/// result identity explicit while still remaining stable for surfaces that
/// want to animate or preserve selection across updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionResultId {
    /// Source slot assigned by the coordinator.
    pub source_slot: SourceSlot,
    /// Source-local result id.
    pub local: LocalResultId,
}

impl SessionResultId {
    /// Create one session-global result id.
    pub const fn new(source_slot: SourceSlot, local: LocalResultId) -> Self {
        Self { source_slot, local }
    }
}

/// Declared behavior of one live source.
///
/// Callers inspect this when they need to know whether a source may revise or
/// retract results, report progress, or be canceled.
///
/// These flags are protocol commitments, not hints. Sources that emit
/// disallowed event kinds are rejected by [`LiveCoordinatorSession`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceCapabilities {
    /// Whether the source may emit results in several rounds.
    pub streams_partial_results: bool,
    /// Whether the source may replace earlier results.
    pub revises_results: bool,
    /// Whether the source may remove earlier results.
    pub retracts_results: bool,
    /// Whether the source may emit progress events.
    pub reports_progress: bool,
    /// Whether the source cooperates with cancellation requests.
    ///
    /// This is a cooperative contract. Sources that set this to `true` should
    /// stop background work promptly, avoid emitting further non-terminal
    /// events, and surface a terminal canceled state when they are polled
    /// directly after cancellation.
    ///
    /// Coordinators may still stop observing non-cancelable sources and mark
    /// them [`SourceState::Stale`] when the surrounding query session is no
    /// longer relevant. They do not prove that source-side work truly halted.
    pub can_cancel: bool,
}

impl SourceCapabilities {
    /// Capabilities for a one-shot lifted snapshot source.
    pub const fn snapshot() -> Self {
        Self {
            streams_partial_results: false,
            revises_results: false,
            retracts_results: false,
            reports_progress: false,
            can_cancel: true,
        }
    }
}

/// Source lifecycle state reported through live status updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceState {
    /// Session has begun and may emit more updates.
    Running,
    /// Source emitted a meaningful partial result set and may continue.
    Partial,
    /// Source completed successfully.
    Complete,
    /// Source failed.
    Failed,
    /// Source was canceled.
    Canceled,
    /// Source data is known to be stale.
    Stale,
}

impl SourceState {
    /// Whether this state is terminal.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Failed | Self::Canceled | Self::Stale
        )
    }
}

/// Status update emitted by a live source or coordinator.
///
/// The `detail` payload is host-defined. Callers often use it for one short
/// error string or a small status explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusUpdate<Detail = ()> {
    /// Source lifecycle state.
    pub state: SourceState,
    /// Optional host-defined detail for the status change.
    pub detail: Option<Detail>,
}

impl<Detail> StatusUpdate<Detail> {
    /// Create one status update with no detail.
    pub const fn new(state: SourceState) -> Self {
        Self {
            state,
            detail: None,
        }
    }

    /// Attach one detail payload to this status update.
    pub fn with_detail(mut self, detail: Detail) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// One source-local result carried by patch operations.
///
/// Sources create this when they insert or replace one hit within a live
/// session.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceResult<S: SubjectRef, A = StandardAffordance, E = ()> {
    /// Stable source-local result id.
    pub id: LocalResultId,
    /// Current hit payload for this result.
    pub hit: PortolanHit<S, A, E>,
}

impl<S: SubjectRef, A, E> SourceResult<S, A, E> {
    /// Create one source-local result.
    pub const fn new(id: LocalResultId, hit: PortolanHit<S, A, E>) -> Self {
        Self { id, hit }
    }
}

/// Source-local patch operation for one live source session.
///
/// Sources emit these inside [`SourceSearchEvent::ApplyPatch`] so callers can
/// update one result list incrementally instead of rebuilding a full snapshot.
#[derive(Clone, Debug, PartialEq)]
pub enum SourcePatchOp<S: SubjectRef, A = StandardAffordance, E = ()> {
    /// Insert one newly visible result.
    Insert {
        /// New result payload.
        result: SourceResult<S, A, E>,
    },
    /// Replace the current payload for one existing result id.
    ///
    /// Replacement preserves the subject already associated with the id while
    /// allowing other hit details to change.
    Replace {
        /// Replacement result payload.
        result: SourceResult<S, A, E>,
    },
    /// Remove one previously visible result.
    Remove {
        /// Source-local result id to remove.
        result_id: LocalResultId,
    },
    /// Move one result before another result, or to the end when `anchor` is `None`.
    MoveBefore {
        /// Result to move.
        result_id: LocalResultId,
        /// Result that should follow the moved result.
        anchor: Option<LocalResultId>,
    },
}

/// Source-local live event emitted by one [`LiveSession`].
#[derive(Clone, Debug, PartialEq)]
pub enum SourceSearchEvent<
    S: SubjectRef,
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
> {
    /// Session has started and declared its capabilities.
    Begin {
        /// Query session id.
        session_id: QuerySessionId,
        /// Source-local revision.
        revision: Revision,
        /// Source capabilities for this session.
        capabilities: SourceCapabilities,
    },
    /// Incremental patch against the source-local result list.
    ApplyPatch {
        /// Query session id.
        session_id: QuerySessionId,
        /// Source-local revision.
        revision: Revision,
        /// Patch operations to apply.
        ops: Vec<SourcePatchOp<S, A, E>>,
    },
    /// Source-local progress update.
    Progress {
        /// Query session id.
        session_id: QuerySessionId,
        /// Source-local revision.
        revision: Revision,
        /// Host-defined progress payload.
        progress: Progress,
    },
    /// Source-local lifecycle change.
    StatusChanged {
        /// Query session id.
        session_id: QuerySessionId,
        /// Source-local revision.
        revision: Revision,
        /// New source status.
        status: StatusUpdate<Detail>,
    },
}

/// Sink for source-local live events.
///
/// Most callers use [`SourceEventBuffer`] unless they need custom buffering or
/// translation behavior.
pub trait SourceEventSink<S: SubjectRef, A = StandardAffordance, E = (), Progress = (), Detail = ()>
{
    /// Push one source-local event into the sink.
    fn push(&mut self, event: SourceSearchEvent<S, A, E, Progress, Detail>);
}

/// Simple growable buffer for source-local live events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceEventBuffer<
    S: SubjectRef,
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
> {
    events: Vec<SourceSearchEvent<S, A, E, Progress, Detail>>,
}

impl<S: SubjectRef, A, E, Progress, Detail> SourceEventBuffer<S, A, E, Progress, Detail> {
    /// Create one empty source event buffer.
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Borrow buffered events.
    pub fn as_slice(&self) -> &[SourceSearchEvent<S, A, E, Progress, Detail>] {
        &self.events
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Remove and return the buffered events.
    pub fn take(&mut self) -> Vec<SourceSearchEvent<S, A, E, Progress, Detail>> {
        core::mem::take(&mut self.events)
    }
}

impl<S: SubjectRef, A, E, Progress, Detail> SourceEventSink<S, A, E, Progress, Detail>
    for SourceEventBuffer<S, A, E, Progress, Detail>
{
    fn push(&mut self, event: SourceSearchEvent<S, A, E, Progress, Detail>) {
        self.events.push(event);
    }
}

/// Outcome of polling a live session or coordinator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollOutcome {
    /// Number of events emitted during this poll.
    pub emitted_events: u32,
    /// Whether the polled session is terminal.
    pub terminal: bool,
}

impl PollOutcome {
    fn with_event_count(emitted_events: usize, terminal: bool) -> Self {
        Self {
            emitted_events: emitted_events
                .try_into()
                .expect("live poll emitted event count overflow"),
            terminal,
        }
    }
}

/// One live source session.
///
/// This trait is the source-local observation seam. Callers poll sessions and
/// drain zero or more [`SourceSearchEvent`] values into a sink until the
/// session reaches a terminal status.
pub trait LiveSession<S: SubjectRef, A = StandardAffordance, E = (), Progress = (), Detail = ()> {
    /// Declared capabilities for this live session.
    fn capabilities(&self) -> SourceCapabilities;

    /// Poll the session and push any newly available source-local events.
    fn poll_events_into(
        &mut self,
        out: &mut dyn SourceEventSink<S, A, E, Progress, Detail>,
    ) -> PollOutcome;

    /// Cancel the session when the surrounding surface no longer needs it.
    ///
    /// Cancellation is cooperative. Implementations that advertise
    /// [`SourceCapabilities::can_cancel`] should treat this as a prompt to stop
    /// ongoing work, suppress further non-terminal updates, and converge on a
    /// terminal canceled state as soon as practical.
    fn cancel(&mut self);
}

/// Source that can open live query sessions.
///
/// The live layer takes owned query and context values so sessions may outlive
/// the initial call that created them.
pub trait LiveSource<
    S: SubjectRef,
    Scope = (),
    Filter = (),
    Context = (),
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
>
{
    /// Open one live retrieval session.
    fn begin_session(
        &self,
        session_id: QuerySessionId,
        query: PortolanQuery<Scope, Filter>,
        context: RetrievalContext<Context>,
        budget: RetrievalBudget,
    ) -> Box<dyn LiveSession<S, A, E, Progress, Detail> + 'static>;
}

/// Live source with an explicit route stage.
///
/// [`LiveCoordinator`] uses this stage to decide the order in which it polls
/// source sessions.
pub trait StagedLiveSource<
    S: SubjectRef,
    Scope = (),
    Filter = (),
    Context = (),
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
>: LiveSource<S, Scope, Filter, Context, A, E, Progress, Detail>
{
    /// Stage in which this source belongs.
    fn stage(&self) -> RouteStage;
}

type LiveSourceRef<'a, S, Scope, Filter, Context, A, E, Progress, Detail> =
    &'a dyn StagedLiveSource<S, Scope, Filter, Context, A, E, Progress, Detail>;

type LabeledLiveSourceList<'a, SourceId, S, Scope, Filter, Context, A, E, Progress, Detail> = [(
    SourceId,
    LiveSourceRef<'a, S, Scope, Filter, Context, A, E, Progress, Detail>,
)];

/// One result in the coordinator-level event stream.
///
/// Coordinated patch operations carry this type so surfaces can render one
/// unified list while still knowing which source and stage produced each hit.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionResult<SourceId, S: SubjectRef, A = StandardAffordance, E = ()> {
    /// Stable session-global result id.
    pub id: SessionResultId,
    /// Caller-provided source identifier.
    pub source: SourceId,
    /// Stage that produced the result.
    pub stage: RouteStage,
    /// Current hit payload.
    pub hit: PortolanHit<S, A, E>,
}

impl<SourceId, S: SubjectRef, A, E> SessionResult<SourceId, S, A, E> {
    fn from_source_result(
        source_slot: SourceSlot,
        source: SourceId,
        stage: RouteStage,
        result: SourceResult<S, A, E>,
    ) -> Self {
        Self {
            id: SessionResultId::new(source_slot, result.id),
            source,
            stage,
            hit: result.hit,
        }
    }
}

/// Patch operation in the coordinator-level event stream.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionPatchOp<SourceId, S: SubjectRef, A = StandardAffordance, E = ()> {
    /// Insert one newly visible result.
    Insert {
        /// New coordinated result payload.
        result: SessionResult<SourceId, S, A, E>,
    },
    /// Replace one previously visible result.
    ///
    /// Coordinated replacements preserve the subject already associated with
    /// the result id while allowing other hit details to change.
    Replace {
        /// Replacement coordinated result payload.
        result: SessionResult<SourceId, S, A, E>,
    },
    /// Remove one previously visible result.
    Remove {
        /// Coordinated result id to remove.
        result_id: SessionResultId,
    },
    /// Move one result before another result, or to the end when `anchor` is `None`.
    MoveBefore {
        /// Result to move.
        result_id: SessionResultId,
        /// Result that should follow the moved result.
        anchor: Option<SessionResultId>,
    },
}

/// Coordinated live event emitted by [`LiveCoordinatorSession`].
#[derive(Clone, Debug, PartialEq)]
pub enum SearchEvent<
    SourceId,
    S: SubjectRef,
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
> {
    /// Coordinator opened one query session.
    SessionStarted {
        /// Query session id.
        session_id: QuerySessionId,
    },
    /// One source session started.
    SourceStarted {
        /// Query session id.
        session_id: QuerySessionId,
        /// Coordinator revision.
        revision: Revision,
        /// Caller-provided source identifier.
        source: SourceId,
        /// Stage associated with the source.
        stage: RouteStage,
        /// Declared capabilities for the source session.
        capabilities: SourceCapabilities,
    },
    /// Coordinator-level patch for one source.
    ApplyPatch {
        /// Query session id.
        session_id: QuerySessionId,
        /// Coordinator revision.
        revision: Revision,
        /// Caller-provided source identifier.
        source: SourceId,
        /// Stage associated with the source.
        stage: RouteStage,
        /// Patch operations to apply.
        ops: Vec<SessionPatchOp<SourceId, S, A, E>>,
    },
    /// Coordinator-level progress update.
    Progress {
        /// Query session id.
        session_id: QuerySessionId,
        /// Coordinator revision.
        revision: Revision,
        /// Caller-provided source identifier.
        source: SourceId,
        /// Stage associated with the source.
        stage: RouteStage,
        /// Host-defined progress payload.
        progress: Progress,
    },
    /// Coordinator-level lifecycle change for one source.
    StatusChanged {
        /// Query session id.
        session_id: QuerySessionId,
        /// Coordinator revision.
        revision: Revision,
        /// Caller-provided source identifier.
        source: SourceId,
        /// Stage associated with the source.
        stage: RouteStage,
        /// New status for the source.
        status: StatusUpdate<Detail>,
    },
    /// Coordinator observed all source sessions reach terminal states.
    SessionFinished {
        /// Query session id.
        session_id: QuerySessionId,
        /// Final coordinator revision.
        revision: Revision,
    },
}

/// Sink for coordinator-level live events.
pub trait SearchEventSink<
    SourceId,
    S: SubjectRef,
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
>
{
    /// Push one coordinated event into the sink.
    fn push(&mut self, event: SearchEvent<SourceId, S, A, E, Progress, Detail>);
}

/// Simple growable buffer for coordinator-level live events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchEventBuffer<
    SourceId,
    S: SubjectRef,
    A = StandardAffordance,
    E = (),
    Progress = (),
    Detail = (),
> {
    events: Vec<SearchEvent<SourceId, S, A, E, Progress, Detail>>,
}

impl<SourceId, S: SubjectRef, A, E, Progress, Detail>
    SearchEventBuffer<SourceId, S, A, E, Progress, Detail>
{
    /// Create one empty event buffer.
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Borrow buffered events.
    pub fn as_slice(&self) -> &[SearchEvent<SourceId, S, A, E, Progress, Detail>] {
        &self.events
    }

    /// Whether the buffer contains no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Remove and return all buffered events.
    pub fn take(&mut self) -> Vec<SearchEvent<SourceId, S, A, E, Progress, Detail>> {
        core::mem::take(&mut self.events)
    }
}

impl<SourceId, S: SubjectRef, A, E, Progress, Detail>
    SearchEventSink<SourceId, S, A, E, Progress, Detail>
    for SearchEventBuffer<SourceId, S, A, E, Progress, Detail>
{
    fn push(&mut self, event: SearchEvent<SourceId, S, A, E, Progress, Detail>) {
        self.events.push(event);
    }
}

/// Lift one cloneable `'static` [`RetrievalSource`] into a live source.
///
/// A successful lifted session emits:
/// - one `Begin` event
/// - one patch event inserting all current hits
/// - one terminal `Complete` status
///
/// If the session is canceled before retrieval runs, it emits `Begin` plus one
/// terminal canceled status instead.
///
/// This is useful when a host wants to mix existing synchronous sources with
/// richer progressive sources under one live coordinator.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotLiveSource<Inner> {
    inner: Inner,
}

impl<Inner> SnapshotLiveSource<Inner> {
    /// Wrap one one-shot retrieval source as a live source.
    pub const fn new(inner: Inner) -> Self {
        Self { inner }
    }

    /// Recover the wrapped one-shot source.
    pub fn into_inner(self) -> Inner {
        self.inner
    }
}

enum SnapshotPhase {
    Ready,
    Finished,
    Canceled,
    Drained,
}

struct SnapshotLiveSession<Inner, S: SubjectRef, Scope, Filter, Context, A, E, Progress, Detail> {
    inner: Inner,
    session_id: QuerySessionId,
    query: PortolanQuery<Scope, Filter>,
    context: RetrievalContext<Context>,
    budget: RetrievalBudget,
    phase: SnapshotPhase,
    _phantom: core::marker::PhantomData<(S, A, E, Progress, Detail)>,
}

impl<Inner, S, Scope, Filter, Context, A, E, Progress, Detail>
    SnapshotLiveSession<Inner, S, Scope, Filter, Context, A, E, Progress, Detail>
where
    S: SubjectRef,
    Inner: RetrievalSource<S, Scope, Filter, Context, A, E>,
{
    fn new(
        inner: Inner,
        session_id: QuerySessionId,
        query: PortolanQuery<Scope, Filter>,
        context: RetrievalContext<Context>,
        budget: RetrievalBudget,
    ) -> Self {
        Self {
            inner,
            session_id,
            query,
            context,
            budget,
            phase: SnapshotPhase::Ready,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<Inner, S, Scope, Filter, Context, A, E, Progress, Detail>
    LiveSession<S, A, E, Progress, Detail>
    for SnapshotLiveSession<Inner, S, Scope, Filter, Context, A, E, Progress, Detail>
where
    S: SubjectRef,
    Inner: RetrievalSource<S, Scope, Filter, Context, A, E>,
{
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::snapshot()
    }

    fn poll_events_into(
        &mut self,
        out: &mut dyn SourceEventSink<S, A, E, Progress, Detail>,
    ) -> PollOutcome {
        match self.phase {
            SnapshotPhase::Ready => {
                out.push(SourceSearchEvent::Begin {
                    session_id: self.session_id,
                    revision: Revision::new(0),
                    capabilities: self.capabilities(),
                });

                let mut buffer = CandidateBuffer::new();
                self.inner
                    .retrieve_into(&self.query, &self.context, self.budget, &mut buffer);

                let ops = buffer
                    .into_hits()
                    .into_iter()
                    .enumerate()
                    .map(|(index, hit)| SourcePatchOp::Insert {
                        result: SourceResult::new(
                            LocalResultId::new(index.try_into().expect("local result id overflow")),
                            hit,
                        ),
                    })
                    .collect();

                out.push(SourceSearchEvent::ApplyPatch {
                    session_id: self.session_id,
                    revision: Revision::new(1),
                    ops,
                });
                out.push(SourceSearchEvent::StatusChanged {
                    session_id: self.session_id,
                    revision: Revision::new(2),
                    status: StatusUpdate::new(SourceState::Complete),
                });
                self.phase = SnapshotPhase::Drained;
                PollOutcome::with_event_count(3, true)
            }
            SnapshotPhase::Finished => PollOutcome::with_event_count(0, true),
            SnapshotPhase::Canceled => {
                out.push(SourceSearchEvent::Begin {
                    session_id: self.session_id,
                    revision: Revision::new(0),
                    capabilities: self.capabilities(),
                });
                out.push(SourceSearchEvent::StatusChanged {
                    session_id: self.session_id,
                    revision: Revision::new(1),
                    status: StatusUpdate::new(SourceState::Canceled),
                });
                self.phase = SnapshotPhase::Finished;
                PollOutcome::with_event_count(2, true)
            }
            SnapshotPhase::Drained => {
                self.phase = SnapshotPhase::Finished;
                PollOutcome::with_event_count(0, true)
            }
        }
    }

    fn cancel(&mut self) {
        if matches!(self.phase, SnapshotPhase::Ready) {
            self.phase = SnapshotPhase::Canceled;
        }
    }
}

impl<Inner, S, Scope, Filter, Context, A, E, Progress, Detail>
    LiveSource<S, Scope, Filter, Context, A, E, Progress, Detail> for SnapshotLiveSource<Inner>
where
    S: SubjectRef,
    S: 'static,
    Scope: 'static,
    Filter: 'static,
    Context: 'static,
    A: 'static,
    E: 'static,
    Progress: 'static,
    Detail: 'static,
    Inner: RetrievalSource<S, Scope, Filter, Context, A, E> + Clone + 'static,
{
    fn begin_session(
        &self,
        session_id: QuerySessionId,
        query: PortolanQuery<Scope, Filter>,
        context: RetrievalContext<Context>,
        budget: RetrievalBudget,
    ) -> Box<dyn LiveSession<S, A, E, Progress, Detail> + 'static> {
        Box::new(SnapshotLiveSession::new(
            self.inner.clone(),
            session_id,
            query,
            context,
            budget,
        ))
    }
}

impl<Inner, S, Scope, Filter, Context, A, E, Progress, Detail>
    StagedLiveSource<S, Scope, Filter, Context, A, E, Progress, Detail>
    for SnapshotLiveSource<Inner>
where
    S: SubjectRef + 'static,
    Scope: 'static,
    Filter: 'static,
    Context: 'static,
    A: 'static,
    E: 'static,
    Progress: 'static,
    Detail: 'static,
    Inner: StagedSnapshotSource<S, Scope, Filter, Context, A, E> + Clone + 'static,
{
    fn stage(&self) -> RouteStage {
        self.inner.stage()
    }
}

/// Lightweight bridge for lifting one-shot routed sources into the live layer.
///
/// Callers usually implement this on the same type that already implements
/// [`RetrievalSource`] when they want to wrap it in [`SnapshotLiveSource`].
pub trait StagedSnapshotSource<
    S: SubjectRef,
    Scope = (),
    Filter = (),
    Context = (),
    A = StandardAffordance,
    E = (),
>: RetrievalSource<S, Scope, Filter, Context, A, E>
{
    /// Route stage for the lifted live source.
    fn stage(&self) -> RouteStage;
}

struct ActiveSession<SourceId, S: SubjectRef, A, E, Progress, Detail> {
    source_slot: SourceSlot,
    source: SourceId,
    stage: RouteStage,
    capabilities: SourceCapabilities,
    session: Box<dyn LiveSession<S, A, E, Progress, Detail> + 'static>,
    protocol: SourceProtocolState<S, Detail>,
}

struct KnownResult<S: SubjectRef> {
    id: LocalResultId,
    subject: S,
}

struct SourceProtocolState<S: SubjectRef, Detail> {
    started: bool,
    announced_started: bool,
    terminal: bool,
    last_revision: Option<Revision>,
    known_results: Vec<KnownResult<S>>,
    pending_status: Option<StatusUpdate<Detail>>,
}

impl<S: SubjectRef, Detail> SourceProtocolState<S, Detail> {
    fn new() -> Self {
        Self {
            started: false,
            announced_started: false,
            terminal: false,
            last_revision: None,
            known_results: Vec::new(),
            pending_status: None,
        }
    }

    fn remember_revision(&mut self, revision: Revision) -> bool {
        if let Some(previous) = self.last_revision
            && revision <= previous
        {
            return false;
        }

        self.last_revision = Some(revision);
        true
    }

    fn contains_result(&self, result_id: LocalResultId) -> bool {
        self.known_results
            .iter()
            .any(|candidate| candidate.id == result_id)
    }

    fn insert_result(&mut self, result_id: LocalResultId, subject: &S) -> bool {
        if self.contains_result(result_id) {
            return false;
        }

        self.known_results.push(KnownResult {
            id: result_id,
            subject: subject.clone(),
        });
        true
    }

    fn replace_result(&self, result_id: LocalResultId, subject: &S) -> bool {
        self.known_results
            .iter()
            .find(|candidate| candidate.id == result_id)
            .is_some_and(|candidate| &candidate.subject == subject)
    }

    fn remove_result(&mut self, result_id: LocalResultId) -> bool {
        if let Some(index) = self
            .known_results
            .iter()
            .position(|candidate| candidate.id == result_id)
        {
            self.known_results.remove(index);
            true
        } else {
            false
        }
    }
}

/// Coordinator for multi-source live sessions.
///
/// This is the main entry point for live Portolan retrieval. It opens one
/// session per source, keeps one session id for the coordinated pass, and
/// translates source-local events into a single coordinated event stream.
#[derive(Debug, Default)]
pub struct LiveCoordinator {
    next_session_id: Cell<u64>,
}

impl LiveCoordinator {
    /// Create one live coordinator with session ids starting at 1.
    pub const fn new() -> Self {
        Self {
            next_session_id: Cell::new(1),
        }
    }

    /// Begin one coordinated live query session.
    pub fn begin_session<SourceId, S, Scope, Filter, Context, A, E, Progress, Detail>(
        &self,
        plan: RoutePlan,
        sources: &LabeledLiveSourceList<
            '_,
            SourceId,
            S,
            Scope,
            Filter,
            Context,
            A,
            E,
            Progress,
            Detail,
        >,
        query: PortolanQuery<Scope, Filter>,
        context: RetrievalContext<Context>,
        budget: RetrievalBudget,
    ) -> LiveCoordinatorSession<SourceId, S, A, E, Progress, Detail>
    where
        SourceId: Clone,
        S: SubjectRef,
        Scope: Clone,
        Filter: Clone,
        Context: Clone,
    {
        let session_id = QuerySessionId::new(self.next_session_id.get());
        self.next_session_id.set(
            self.next_session_id
                .get()
                .checked_add(1)
                .expect("query session id overflow"),
        );

        let mut active = Vec::new();
        let mut next_slot = 0_u16;
        for stage in plan.stages() {
            for (source_id, source) in sources {
                if source.stage() != *stage {
                    continue;
                }

                let session =
                    source.begin_session(session_id, query.clone(), context.clone(), budget);
                let capabilities = session.capabilities();
                active.push(ActiveSession {
                    source_slot: SourceSlot::new(next_slot),
                    source: source_id.clone(),
                    stage: *stage,
                    capabilities,
                    session,
                    protocol: SourceProtocolState::new(),
                });
                next_slot = next_slot.checked_add(1).expect("source slot overflow");
            }
        }

        LiveCoordinatorSession {
            session_id,
            next_revision: 0,
            session_started: false,
            session_finished: false,
            active,
        }
    }
}

/// One coordinated live query session.
///
/// Callers poll this type repeatedly, usually into a [`SearchEventBuffer`],
/// until [`PollOutcome::terminal`] becomes `true`.
pub struct LiveCoordinatorSession<SourceId, S: SubjectRef, A, E, Progress, Detail> {
    session_id: QuerySessionId,
    next_revision: u64,
    session_started: bool,
    session_finished: bool,
    active: Vec<ActiveSession<SourceId, S, A, E, Progress, Detail>>,
}

impl<SourceId, S: SubjectRef, A, E, Progress, Detail> fmt::Debug
    for LiveCoordinatorSession<SourceId, S, A, E, Progress, Detail>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveCoordinatorSession")
            .field("session_id", &self.session_id)
            .field("next_revision", &self.next_revision)
            .field("session_started", &self.session_started)
            .field("session_finished", &self.session_finished)
            .field("active_sessions", &self.active.len())
            .finish()
    }
}

impl<SourceId, S: SubjectRef, A, E, Progress, Detail>
    LiveCoordinatorSession<SourceId, S, A, E, Progress, Detail>
where
    SourceId: Clone,
{
    /// Session id for this coordinated pass.
    pub const fn session_id(&self) -> QuerySessionId {
        self.session_id
    }

    /// Cancel every still-running source session.
    ///
    /// Sources that report [`SourceCapabilities::can_cancel`] become
    /// [`SourceState::Canceled`]. Sources that do not cooperate with
    /// cancellation are marked [`SourceState::Stale`] instead so the
    /// coordinator can stop observing them without claiming a stronger
    /// guarantee than the source provides.
    ///
    /// This is still a cooperative process. The coordinator records the
    /// cancellation outcome in the coordinated event stream and stops
    /// observing the source after the terminal status is emitted, but it does
    /// not prove that source-side work really stopped.
    pub fn cancel(&mut self) {
        for active in &mut self.active {
            if !active.protocol.terminal && active.protocol.pending_status.is_none() {
                if active.capabilities.can_cancel {
                    active.session.cancel();
                    active.protocol.pending_status = Some(StatusUpdate::new(SourceState::Canceled));
                } else {
                    active.protocol.pending_status = Some(StatusUpdate::new(SourceState::Stale));
                }
            }
        }
    }

    /// Poll all live sources once in stage order and translate their updates.
    pub fn poll_events_into(
        &mut self,
        out: &mut dyn SearchEventSink<SourceId, S, A, E, Progress, Detail>,
    ) -> PollOutcome {
        let mut emitted = 0_usize;

        if !self.session_started {
            out.push(SearchEvent::SessionStarted {
                session_id: self.session_id,
            });
            self.session_started = true;
            emitted += 1;
        }

        for index in 0..self.active.len() {
            if self.active[index].protocol.terminal {
                continue;
            }

            let pending_status = {
                let active = &mut self.active[index];
                active.protocol.pending_status.take()
            };
            if let Some(status) = pending_status {
                if !self.active[index].protocol.announced_started {
                    emitted += 1;
                    let revision = self.bump_revision();
                    self.active[index].protocol.announced_started = true;
                    out.push(SearchEvent::SourceStarted {
                        session_id: self.session_id,
                        revision,
                        source: self.active[index].source.clone(),
                        stage: self.active[index].stage,
                        capabilities: self.active[index].capabilities,
                    });
                }

                emitted += 1;
                let revision = self.bump_revision();
                if status.state.is_terminal() {
                    self.active[index].protocol.terminal = true;
                }
                out.push(SearchEvent::StatusChanged {
                    session_id: self.session_id,
                    revision,
                    source: self.active[index].source.clone(),
                    stage: self.active[index].stage,
                    status,
                });
                continue;
            }

            let (source_slot, source, stage, poll, source_events) = {
                let active = &mut self.active[index];
                let mut source_events = SourceEventBuffer::new();
                let poll = active.session.poll_events_into(&mut source_events);
                (
                    active.source_slot,
                    active.source.clone(),
                    active.stage,
                    poll,
                    source_events.take(),
                )
            };

            for event in source_events {
                let capabilities = self.active[index].capabilities;
                let validation = validate_source_event(
                    &mut self.active[index].protocol,
                    capabilities,
                    self.session_id,
                    &event,
                );
                if validation.is_err() {
                    emitted += 1;
                    let revision = self.bump_revision();
                    self.active[index].protocol.terminal = true;
                    out.push(SearchEvent::StatusChanged {
                        session_id: self.session_id,
                        revision,
                        source: source.clone(),
                        stage,
                        status: StatusUpdate::new(SourceState::Failed),
                    });
                    break;
                }

                if matches!(event, SourceSearchEvent::Begin { .. }) {
                    self.active[index].protocol.announced_started = true;
                }

                emitted += 1;
                out.push(translate_event(
                    self.session_id,
                    &mut self.next_revision,
                    source_slot,
                    source.clone(),
                    stage,
                    event,
                ));
            }

            if poll.terminal && !self.active[index].protocol.terminal {
                emitted += 1;
                let revision = self.bump_revision();
                self.active[index].protocol.terminal = true;
                out.push(SearchEvent::StatusChanged {
                    session_id: self.session_id,
                    revision,
                    source,
                    stage,
                    status: StatusUpdate::new(SourceState::Failed),
                });
            }
        }

        let terminal = self.active.iter().all(|active| active.protocol.terminal);
        if terminal && !self.session_finished {
            let revision = self.bump_revision();
            out.push(SearchEvent::SessionFinished {
                session_id: self.session_id,
                revision,
            });
            self.session_finished = true;
            emitted += 1;
        }

        PollOutcome::with_event_count(emitted, terminal)
    }

    fn bump_revision(&mut self) -> Revision {
        let revision = Revision::new(self.next_revision);
        self.next_revision = self
            .next_revision
            .checked_add(1)
            .expect("coordinator revision overflow");
        revision
    }
}

fn validate_source_event<S: SubjectRef, A, E, Progress, Detail>(
    state: &mut SourceProtocolState<S, Detail>,
    capabilities: SourceCapabilities,
    session_id: QuerySessionId,
    event: &SourceSearchEvent<S, A, E, Progress, Detail>,
) -> Result<(), ()> {
    let (event_session_id, event_revision) = match event {
        SourceSearchEvent::Begin {
            session_id,
            revision,
            ..
        }
        | SourceSearchEvent::ApplyPatch {
            session_id,
            revision,
            ..
        }
        | SourceSearchEvent::Progress {
            session_id,
            revision,
            ..
        }
        | SourceSearchEvent::StatusChanged {
            session_id,
            revision,
            ..
        } => (*session_id, *revision),
    };

    if event_session_id != session_id {
        return Err(());
    }

    if state.terminal || !state.remember_revision(event_revision) {
        return Err(());
    }

    match event {
        SourceSearchEvent::Begin { .. } => {
            if state.started {
                return Err(());
            }
            state.started = true;
        }
        SourceSearchEvent::ApplyPatch { ops, .. } => {
            if !state.started {
                return Err(());
            }

            for op in ops {
                match op {
                    SourcePatchOp::Insert { result } => {
                        if !state.insert_result(result.id, &result.hit.subject) {
                            return Err(());
                        }
                    }
                    SourcePatchOp::Replace { result } => {
                        if !capabilities.revises_results {
                            return Err(());
                        }
                        if !state.replace_result(result.id, &result.hit.subject) {
                            return Err(());
                        }
                    }
                    SourcePatchOp::Remove { result_id } => {
                        if !capabilities.retracts_results {
                            return Err(());
                        }
                        if !state.remove_result(*result_id) {
                            return Err(());
                        }
                    }
                    SourcePatchOp::MoveBefore { result_id, anchor } => {
                        if !capabilities.revises_results {
                            return Err(());
                        }
                        if !state.contains_result(*result_id) {
                            return Err(());
                        }
                        if let Some(anchor) = anchor
                            && (!state.contains_result(*anchor) || anchor == result_id)
                        {
                            return Err(());
                        }
                    }
                }
            }
        }
        SourceSearchEvent::Progress { .. } => {
            if !state.started {
                return Err(());
            }
            if !capabilities.reports_progress {
                return Err(());
            }
        }
        SourceSearchEvent::StatusChanged { status, .. } => {
            if !state.started {
                return Err(());
            }
            if status.state.is_terminal() {
                state.terminal = true;
            }
        }
    }

    Ok(())
}

fn translate_event<SourceId, S: SubjectRef, A, E, Progress, Detail>(
    session_id: QuerySessionId,
    next_revision: &mut u64,
    source_slot: SourceSlot,
    source: SourceId,
    stage: RouteStage,
    event: SourceSearchEvent<S, A, E, Progress, Detail>,
) -> SearchEvent<SourceId, S, A, E, Progress, Detail>
where
    SourceId: Clone,
{
    let revision = Revision::new(*next_revision);
    *next_revision = next_revision
        .checked_add(1)
        .expect("coordinator revision overflow");

    match event {
        SourceSearchEvent::Begin { capabilities, .. } => SearchEvent::SourceStarted {
            session_id,
            revision,
            source,
            stage,
            capabilities,
        },
        SourceSearchEvent::ApplyPatch { ops, .. } => SearchEvent::ApplyPatch {
            session_id,
            revision,
            source: source.clone(),
            stage,
            ops: ops
                .into_iter()
                .map(|op| translate_patch_op(source_slot, source.clone(), stage, op))
                .collect(),
        },
        SourceSearchEvent::Progress { progress, .. } => SearchEvent::Progress {
            session_id,
            revision,
            source,
            stage,
            progress,
        },
        SourceSearchEvent::StatusChanged { status, .. } => SearchEvent::StatusChanged {
            session_id,
            revision,
            source,
            stage,
            status,
        },
    }
}

fn translate_patch_op<SourceId, S: SubjectRef, A, E>(
    source_slot: SourceSlot,
    source: SourceId,
    stage: RouteStage,
    op: SourcePatchOp<S, A, E>,
) -> SessionPatchOp<SourceId, S, A, E> {
    match op {
        SourcePatchOp::Insert { result } => SessionPatchOp::Insert {
            result: SessionResult::from_source_result(source_slot, source, stage, result),
        },
        SourcePatchOp::Replace { result } => SessionPatchOp::Replace {
            result: SessionResult::from_source_result(source_slot, source, stage, result),
        },
        SourcePatchOp::Remove { result_id } => SessionPatchOp::Remove {
            result_id: SessionResultId::new(source_slot, result_id),
        },
        SourcePatchOp::MoveBefore { result_id, anchor } => SessionPatchOp::MoveBefore {
            result_id: SessionResultId::new(source_slot, result_id),
            anchor: anchor.map(|anchor| SessionResultId::new(source_slot, anchor)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LiveCoordinator, LiveSession, LiveSource, LocalResultId, QuerySessionId, Revision,
        SearchEvent, SearchEventBuffer, SnapshotLiveSource, SourceCapabilities, SourceEventBuffer,
        SourcePatchOp, SourceResult, SourceSearchEvent, SourceState, StagedLiveSource,
        StagedSnapshotSource, StatusUpdate,
    };
    use alloc::boxed::Box;
    use alloc::vec;
    use alloc::vec::Vec;
    use portolan_core::{Affordance, PortolanHit, RetrievalContext, RetrievalOrigin, Score};
    use portolan_query::PortolanQuery;
    use portolan_route::{RoutePlan, RouteStage};
    use portolan_source::{CandidateSink, RetrievalSource};

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct DemoSubject(&'static str);

    #[derive(Clone, Copy, Debug)]
    struct DemoSnapshotSource;

    impl RetrievalSource<DemoSubject> for DemoSnapshotSource {
        fn retrieve_into(
            &self,
            _query: &PortolanQuery,
            _context: &RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
            out: &mut dyn CandidateSink<DemoSubject>,
        ) {
            out.push(PortolanHit::new(
                DemoSubject("camera.main"),
                Score::new(1.0),
                RetrievalOrigin::MaterializedIndex,
            ));
        }
    }

    impl StagedSnapshotSource<DemoSubject> for DemoSnapshotSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Materialized
        }
    }

    struct ProgressiveSource;

    struct MalformedPatchSource;

    struct MalformedPatchSession {
        session_id: QuerySessionId,
    }

    struct StaleRevisionSource;

    struct StaleRevisionSession {
        session_id: QuerySessionId,
    }

    struct UnknownReplaceSource;

    struct UnknownReplaceSession {
        session_id: QuerySessionId,
    }

    struct WrongSessionIdSource;

    struct WrongSessionIdSession {
        session_id: QuerySessionId,
    }

    struct InvalidMoveSource;

    struct InvalidMoveSession {
        session_id: QuerySessionId,
    }

    struct ReplaceWithoutCapabilitySource;

    struct ReplaceWithoutCapabilitySession {
        session_id: QuerySessionId,
    }

    struct RemoveWithoutCapabilitySource;

    struct RemoveWithoutCapabilitySession {
        session_id: QuerySessionId,
    }

    struct ProgressWithoutCapabilitySource;

    struct ProgressWithoutCapabilitySession {
        session_id: QuerySessionId,
    }

    struct SubjectChangingReplaceSource;

    struct SubjectChangingReplaceSession {
        session_id: QuerySessionId,
    }

    struct NonCancelableSource;

    struct NonCancelableSession {
        session_id: QuerySessionId,
    }

    struct IgnoringCancelSource;

    struct IgnoringCancelSession {
        session_id: QuerySessionId,
        canceled: bool,
    }

    struct ProgressiveSession {
        session_id: QuerySessionId,
        step: u8,
        canceled: bool,
    }

    impl LiveSession<DemoSubject> for ProgressiveSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: true,
                retracts_results: true,
                reports_progress: true,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            if self.canceled {
                out.push(SourceSearchEvent::StatusChanged {
                    session_id: self.session_id,
                    revision: Revision::new(99),
                    status: StatusUpdate::new(SourceState::Canceled),
                });
                self.canceled = false;
                return super::PollOutcome::with_event_count(1, true);
            }

            match self.step {
                0 => {
                    out.push(SourceSearchEvent::Begin {
                        session_id: self.session_id,
                        revision: Revision::new(0),
                        capabilities: self.capabilities(),
                    });
                    self.step = 1;
                    super::PollOutcome::with_event_count(1, false)
                }
                1 => {
                    out.push(SourceSearchEvent::ApplyPatch {
                        session_id: self.session_id,
                        revision: Revision::new(1),
                        ops: vec![SourcePatchOp::Insert {
                            result: SourceResult::new(
                                LocalResultId::new(4),
                                PortolanHit::new(
                                    DemoSubject("camera.secondary"),
                                    Score::new(0.4),
                                    RetrievalOrigin::VirtualScan,
                                ),
                            ),
                        }],
                    });
                    self.step = 2;
                    super::PollOutcome::with_event_count(1, false)
                }
                2 => {
                    out.push(SourceSearchEvent::ApplyPatch {
                        session_id: self.session_id,
                        revision: Revision::new(2),
                        ops: vec![SourcePatchOp::Replace {
                            result: SourceResult::new(
                                LocalResultId::new(4),
                                PortolanHit::new(
                                    DemoSubject("camera.secondary"),
                                    Score::new(0.9),
                                    RetrievalOrigin::VirtualScan,
                                )
                                .with_affordances(Vec::from([
                                    Affordance::new(portolan_core::StandardAffordance::Inspect),
                                ])),
                            ),
                        }],
                    });
                    self.step = 3;
                    super::PollOutcome::with_event_count(1, false)
                }
                3 => {
                    out.push(SourceSearchEvent::StatusChanged {
                        session_id: self.session_id,
                        revision: Revision::new(3),
                        status: StatusUpdate::new(SourceState::Complete),
                    });
                    self.step = 4;
                    super::PollOutcome::with_event_count(1, true)
                }
                _ => super::PollOutcome::with_event_count(0, true),
            }
        }

        fn cancel(&mut self) {
            self.canceled = true;
        }
    }

    impl LiveSource<DemoSubject> for ProgressiveSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(ProgressiveSession {
                session_id,
                step: 0,
                canceled: false,
            })
        }
    }

    impl StagedLiveSource<DemoSubject> for ProgressiveSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for MalformedPatchSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities::snapshot()
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(0),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(0),
                        PortolanHit::new(
                            DemoSubject("camera.bad"),
                            Score::new(0.1),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(1, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for MalformedPatchSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(MalformedPatchSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for MalformedPatchSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for StaleRevisionSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: true,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(3),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(2),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(0),
                        PortolanHit::new(
                            DemoSubject("camera.stale"),
                            Score::new(0.2),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(2, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for StaleRevisionSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(StaleRevisionSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for StaleRevisionSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for UnknownReplaceSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: true,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Replace {
                    result: SourceResult::new(
                        LocalResultId::new(9),
                        PortolanHit::new(
                            DemoSubject("camera.unknown"),
                            Score::new(0.7),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(2, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for UnknownReplaceSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(UnknownReplaceSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for UnknownReplaceSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for WrongSessionIdSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities::snapshot()
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: QuerySessionId::new(self.session_id.get().saturating_add(1)),
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            super::PollOutcome::with_event_count(1, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for WrongSessionIdSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(WrongSessionIdSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for WrongSessionIdSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for InvalidMoveSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: true,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(0),
                        PortolanHit::new(
                            DemoSubject("camera.movable"),
                            Score::new(0.4),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(2),
                ops: vec![SourcePatchOp::MoveBefore {
                    result_id: LocalResultId::new(0),
                    anchor: Some(LocalResultId::new(0)),
                }],
            });
            super::PollOutcome::with_event_count(3, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for InvalidMoveSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(InvalidMoveSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for InvalidMoveSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for ReplaceWithoutCapabilitySession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: false,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(5),
                        PortolanHit::new(
                            DemoSubject("camera.replaceable"),
                            Score::new(0.2),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(2),
                ops: vec![SourcePatchOp::Replace {
                    result: SourceResult::new(
                        LocalResultId::new(5),
                        PortolanHit::new(
                            DemoSubject("camera.replaceable"),
                            Score::new(0.8),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(3, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for ReplaceWithoutCapabilitySource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(ReplaceWithoutCapabilitySession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for ReplaceWithoutCapabilitySource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for RemoveWithoutCapabilitySession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: false,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(6),
                        PortolanHit::new(
                            DemoSubject("camera.removable"),
                            Score::new(0.3),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(2),
                ops: vec![SourcePatchOp::Remove {
                    result_id: LocalResultId::new(6),
                }],
            });
            super::PollOutcome::with_event_count(3, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for RemoveWithoutCapabilitySource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(RemoveWithoutCapabilitySession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for RemoveWithoutCapabilitySource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for ProgressWithoutCapabilitySession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: false,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::Progress {
                session_id: self.session_id,
                revision: Revision::new(1),
                progress: (),
            });
            super::PollOutcome::with_event_count(2, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for ProgressWithoutCapabilitySource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(ProgressWithoutCapabilitySession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for ProgressWithoutCapabilitySource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for SubjectChangingReplaceSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: true,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(2),
                        PortolanHit::new(
                            DemoSubject("camera.original"),
                            Score::new(0.4),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(2),
                ops: vec![SourcePatchOp::Replace {
                    result: SourceResult::new(
                        LocalResultId::new(2),
                        PortolanHit::new(
                            DemoSubject("camera.changed"),
                            Score::new(0.9),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(3, true)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for SubjectChangingReplaceSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(SubjectChangingReplaceSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for SubjectChangingReplaceSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for NonCancelableSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: false,
                retracts_results: false,
                reports_progress: false,
                can_cancel: false,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(0),
                        PortolanHit::new(
                            DemoSubject("camera.non_cancelable"),
                            Score::new(0.2),
                            RetrievalOrigin::VirtualScan,
                        ),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(2, false)
        }

        fn cancel(&mut self) {}
    }

    impl LiveSource<DemoSubject> for NonCancelableSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(NonCancelableSession { session_id })
        }
    }

    impl StagedLiveSource<DemoSubject> for NonCancelableSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    impl LiveSession<DemoSubject> for IgnoringCancelSession {
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities {
                streams_partial_results: true,
                revises_results: false,
                retracts_results: false,
                reports_progress: false,
                can_cancel: true,
            }
        }

        fn poll_events_into(
            &mut self,
            out: &mut dyn super::SourceEventSink<DemoSubject>,
        ) -> super::PollOutcome {
            let subject = if self.canceled {
                DemoSubject("camera.after_cancel")
            } else {
                DemoSubject("camera.before_cancel")
            };
            out.push(SourceSearchEvent::Begin {
                session_id: self.session_id,
                revision: Revision::new(0),
                capabilities: self.capabilities(),
            });
            out.push(SourceSearchEvent::ApplyPatch {
                session_id: self.session_id,
                revision: Revision::new(1),
                ops: vec![SourcePatchOp::Insert {
                    result: SourceResult::new(
                        LocalResultId::new(0),
                        PortolanHit::new(subject, Score::new(0.3), RetrievalOrigin::VirtualScan),
                    ),
                }],
            });
            super::PollOutcome::with_event_count(2, false)
        }

        fn cancel(&mut self) {
            self.canceled = true;
        }
    }

    impl LiveSource<DemoSubject> for IgnoringCancelSource {
        fn begin_session(
            &self,
            session_id: QuerySessionId,
            _query: PortolanQuery,
            _context: RetrievalContext,
            _budget: portolan_core::RetrievalBudget,
        ) -> Box<dyn LiveSession<DemoSubject> + 'static> {
            Box::new(IgnoringCancelSession {
                session_id,
                canceled: false,
            })
        }
    }

    impl StagedLiveSource<DemoSubject> for IgnoringCancelSource {
        fn stage(&self) -> RouteStage {
            RouteStage::Virtual
        }
    }

    #[test]
    fn snapshot_live_source_emits_begin_patch_and_complete() {
        let source = SnapshotLiveSource::new(DemoSnapshotSource);
        let mut session: Box<dyn LiveSession<DemoSubject>> = source.begin_session(
            QuerySessionId::new(1),
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SourceEventBuffer::new();
        let outcome = session.poll_events_into(&mut buffer);

        assert_eq!(outcome.emitted_events, 3);
        assert!(outcome.terminal);
        assert!(matches!(
            &buffer.as_slice()[0],
            SourceSearchEvent::Begin {
                capabilities,
                ..
            } if *capabilities == SourceCapabilities::snapshot()
        ));
        assert!(matches!(
            &buffer.as_slice()[1],
            SourceSearchEvent::ApplyPatch { ops, .. }
                if matches!(
                    &ops[0],
                    SourcePatchOp::Insert { result }
                        if result.id == LocalResultId::new(0)
                        && result.hit.subject == DemoSubject("camera.main")
                )
        ));
        assert!(matches!(
            &buffer.as_slice()[2],
            SourceSearchEvent::StatusChanged { status, .. }
                if status.state == SourceState::Complete
        ));
    }

    #[test]
    fn canceled_snapshot_live_source_emits_begin_then_canceled() {
        let source = SnapshotLiveSource::new(DemoSnapshotSource);
        let mut session: Box<dyn LiveSession<DemoSubject>> = source.begin_session(
            QuerySessionId::new(1),
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        session.cancel();
        let mut buffer = SourceEventBuffer::new();
        let outcome = session.poll_events_into(&mut buffer);

        assert_eq!(outcome.emitted_events, 2);
        assert!(outcome.terminal);
        assert!(matches!(
            buffer.as_slice()[0],
            SourceSearchEvent::Begin { .. }
        ));
        assert!(matches!(
            &buffer.as_slice()[1],
            SourceSearchEvent::StatusChanged { status, .. }
                if status.state == SourceState::Canceled
        ));
    }

    #[test]
    fn coordinator_translates_source_local_ids_into_session_ids() {
        let snapshot = SnapshotLiveSource::new(DemoSnapshotSource);
        let progressive = ProgressiveSource;
        let sources = [
            (
                "materialized",
                &snapshot as &dyn StagedLiveSource<DemoSubject>,
            ),
            (
                "virtual",
                &progressive as &dyn StagedLiveSource<DemoSubject>,
            ),
        ];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let first = session.poll_events_into(&mut buffer);
        assert!(!first.terminal);

        let second = session.poll_events_into(&mut buffer);
        assert!(!second.terminal);

        let third = session.poll_events_into(&mut buffer);
        assert!(!third.terminal);

        let fourth = session.poll_events_into(&mut buffer);
        assert!(fourth.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[0],
            SearchEvent::SessionStarted { session_id }
                if *session_id == QuerySessionId::new(1)
        ));
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, stage, .. }
                if *source == "materialized" && *stage == RouteStage::Materialized
        ));
        assert!(matches!(
            &events[2],
            SearchEvent::ApplyPatch { source, ops, .. }
                if *source == "materialized"
                    && matches!(
                        &ops[0],
                        super::SessionPatchOp::Insert { result }
                            if result.id
                                == super::SessionResultId::new(
                                    super::SourceSlot::new(0),
                                    LocalResultId::new(0)
                                )
                    )
        ));
        assert!(matches!(
            events.last(),
            Some(SearchEvent::SessionFinished { session_id, .. })
                if *session_id == QuerySessionId::new(1)
        ));
    }

    #[test]
    fn coordinator_fails_source_that_patches_before_begin() {
        let malformed = MalformedPatchSource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(events[0], SearchEvent::SessionStarted { .. }));
        assert!(matches!(
            &events[1],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
        assert!(matches!(events[2], SearchEvent::SessionFinished { .. }));
    }

    #[test]
    fn coordinator_fails_source_on_stale_revision() {
        let malformed = StaleRevisionSource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[2],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_on_unknown_replace_id() {
        let malformed = UnknownReplaceSource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[2],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_on_mismatched_session_id() {
        let malformed = WrongSessionIdSource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_on_invalid_move_anchor() {
        let malformed = InvalidMoveSource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[3],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_on_subject_changing_replace() {
        let malformed = SubjectChangingReplaceSource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[3],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_when_replace_violates_capabilities() {
        let malformed = ReplaceWithoutCapabilitySource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[3],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_when_remove_violates_capabilities() {
        let malformed = RemoveWithoutCapabilitySource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[3],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn coordinator_fails_source_when_progress_violates_capabilities() {
        let malformed = ProgressWithoutCapabilitySource;
        let sources = [("virtual", &malformed as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[2],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Failed
        ));
    }

    #[test]
    fn cancel_marks_non_cooperative_source_canceled() {
        let source = IgnoringCancelSource;
        let sources = [("virtual", &source as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let first = session.poll_events_into(&mut buffer);
        assert!(!first.terminal);
        session.cancel();

        let second = session.poll_events_into(&mut buffer);
        assert!(second.terminal);

        let events = buffer.take();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                SearchEvent::StatusChanged { source, status, .. }
                    if *source == "virtual" && status.state == SourceState::Canceled
            )
        }));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, SearchEvent::SessionFinished { .. }))
        );
    }

    #[test]
    fn cancel_before_first_poll_emits_source_started_then_canceled() {
        let source = IgnoringCancelSource;
        let sources = [("virtual", &source as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        session.cancel();
        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(events[0], SearchEvent::SessionStarted { .. }));
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[2],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Canceled
        ));
        assert!(matches!(events[3], SearchEvent::SessionFinished { .. }));
    }

    #[test]
    fn cancel_marks_non_cancelable_source_stale() {
        let source = NonCancelableSource;
        let sources = [("virtual", &source as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        let first = session.poll_events_into(&mut buffer);
        assert!(!first.terminal);
        session.cancel();

        let second = session.poll_events_into(&mut buffer);
        assert!(second.terminal);

        let events = buffer.take();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                SearchEvent::StatusChanged { source, status, .. }
                    if *source == "virtual" && status.state == SourceState::Stale
            )
        }));
    }

    #[test]
    fn cancel_before_first_poll_emits_source_started_then_stale() {
        let source = NonCancelableSource;
        let sources = [("virtual", &source as &dyn StagedLiveSource<DemoSubject>)];
        let coordinator = LiveCoordinator::new();
        let mut session = coordinator.begin_session(
            RoutePlan::standard(),
            &sources,
            PortolanQuery::<(), ()>::text("camera"),
            RetrievalContext::default(),
            portolan_core::RetrievalBudget::interactive_default(),
        );
        let mut buffer = SearchEventBuffer::new();

        session.cancel();
        let poll = session.poll_events_into(&mut buffer);
        assert!(poll.terminal);

        let events = buffer.take();
        assert!(matches!(events[0], SearchEvent::SessionStarted { .. }));
        assert!(matches!(
            &events[1],
            SearchEvent::SourceStarted { source, .. } if *source == "virtual"
        ));
        assert!(matches!(
            &events[2],
            SearchEvent::StatusChanged { source, status, .. }
                if *source == "virtual" && status.state == SourceState::Stale
        ));
        assert!(matches!(events[3], SearchEvent::SessionFinished { .. }));
    }
}
