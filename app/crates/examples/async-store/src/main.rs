//! Making an **async, authoritative store** durable through ironstate without
//! implementing the synchronous [`Journal`](ironstate_journal::Journal) trait.
//!
//! This is the recollect shape: the source of truth is a database reached over an
//! async client (tokio-postgres, say). Its `append`/read operations are `async fn`,
//! so it cannot implement ironstate's synchronous `Journal` — that trait's
//! `fn append(&mut self, …) -> Result<Seq, JournalError>` has no place to `.await`.
//! The reflex fixes are unappealing: a blocking client behind `spawn_blocking`
//! (a second, sync connection pool beside the async one), or a `block_on` bridge
//! that re-enters the runtime (an anti-pattern).
//!
//! The clean path keeps the async stack and still gets the durability guarantee,
//! because the load-bearing discipline of the persistent loop is not in the
//! `Journal` trait — it is in three pure steps ironstate exposes:
//!
//! - [`prepare`] — structural checks, `decide`, and the entropy-position capture,
//!   with rewind-on-rejection. Pure; touches no storage.
//! - [`Prepared::commit`](ironstate_journal::Prepared::commit) — evolve the
//!   aggregate after the append succeeds.
//! - [`Prepared::abort`](ironstate_journal::Prepared::abort) — rewind the entropy
//!   stream after the append fails.
//!
//! A consumer owns only the IO: read the head position, `prepare`, **`.await` its
//! own append**, then `commit` or `abort`. There is exactly one mutating `.await`
//! and no entropy/ordering logic to copy, so the async loop cannot drift from the
//! built-in [`execute`](ironstate_journal::execute). [`resume`](ironstate_journal::resume)
//! has the same shape: this example's async resume reads the store with `.await`
//! and feeds the pure [`replay`] primitive.
//!
//! **Keeping the durable path under the contract.** The catch with rolling your own
//! loop is that the storage — the part that can actually corrupt durability — sits
//! outside `journal_contract_test!`. The fix is a *synchronous twin*: a `Journal`
//! over the very same storage (here `SyncStore` over [`Log`]) that exists only to
//! be measured by the seven-property suite. Production drives the storage through
//! the async front end ([`AsyncStore`]); the contract proves the storage semantics
//! through the sync one. Same `Log`, two front ends — so "we went async" never means
//! "we left the yardstick behind."

use std::borrow::Cow;

use anyhow::{Result, anyhow};
use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateRules, CtxEntropy, DrawPos, EntropySource, LogicalTime,
    OwnedDeterministicCtx, Seed, SeededEntropy, StableHash,
};
use ironstate_journal::{
    ExecuteError, JournalError, ResumeError, Seq, Snapshot, VersionedEvent, prepare, replay,
};
// The sync twin and the contract macro that measures it are test-only — production
// never constructs a `SyncStore`.
#[cfg(test)]
use ironstate_journal::{ContractJournal, Journal};
use proptest::prelude::*;

// === a minimal async runtime ==============================================
//
// A ~20-line stand-in for `#[tokio::main]`, so the example pulls in no runtime
// dependency. recollect would delete this module and use its real runtime.
mod rt {
    use std::future::Future;
    use std::pin::{Pin, pin};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread::{self, Thread};

    /// Drive a future to completion on the current thread, parking between polls.
    pub fn block_on<F: Future>(future: F) -> F::Output {
        struct ThreadWaker(Thread);
        impl Wake for ThreadWaker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(output) => return output,
                Poll::Pending => thread::park(),
            }
        }
    }

    /// A one-shot await point, modeling the suspension at a real DB round-trip so
    /// the loop genuinely awaits rather than running straight through.
    pub fn yield_now() -> impl Future<Output = ()> {
        struct YieldOnce(bool);
        impl Future for YieldOnce {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.0 {
                    Poll::Ready(())
                } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }
        YieldOnce(false)
    }
}

// === the aggregate: a tally that draws a random increment per roll ==========

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Live, terminal = [Sealed])]
enum Phase {
    Live,
    Sealed,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum PhaseStep {
    Seal,
}

impl TransitionRules for Phase {
    type Event = PhaseStep;
    fn transition(&self, _: &PhaseStep) -> Option<Phase> {
        matches!(self, Phase::Live).then_some(Phase::Sealed)
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Roll,
    Seal,
}

#[derive(Clone, Debug, PartialEq)]
enum TallyEvent {
    Rolled(u8),
    Sealed,
}

#[derive(Debug, thiserror::Error)]
#[error("the tally is sealed")]
struct SealedError;

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Tally {
    phase: Phase,
    total: u32,
}

/// The genesis (pre-append) state.
fn genesis() -> Tally {
    Tally {
        phase: Phase::Live,
        total: 0,
    }
}

impl AggregateRules for Tally {
    type Phase = Phase;
    type Command = Command;
    type Event = TallyEvent;
    type Error = SealedError;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<TallyEvent>, SealedError> {
        if self.phase != Phase::Live {
            return Err(SealedError);
        }
        Ok(match cmd {
            // The only draw — replay reproduces it from the recorded position.
            Command::Roll => vec![TallyEvent::Rolled(ctx.entropy.draw_range(1..7) as u8)],
            Command::Seal => vec![TallyEvent::Sealed],
        })
    }

    fn evolve(&mut self, event: &TallyEvent) {
        match event {
            TallyEvent::Rolled(n) => self.total += u32::from(*n),
            TallyEvent::Sealed => self.phase = Phase::Sealed,
        }
    }
}

impl AggregateArbitrary for Tally {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(genesis()).boxed()
    }
    fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
        // Mostly rolls (which draw and advance positions), occasionally a seal.
        prop_oneof![8 => Just(Command::Roll), 1 => Just(Command::Seal)].boxed()
    }
    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        OwnedDeterministicCtx {
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

// === the storage: one log, two front ends ==================================

struct Record<A: AggregateRules> {
    events: Vec<A::Event>,
    entropy_pos: DrawPos,
}

/// The durable log, modeled in memory: append-only records (a batch of events plus
/// the entropy position the batch consumed, stored as one atomic unit) and a
/// genesis snapshot. This stands in for the database table an adapter author
/// writes; both front ends drive *this*, so proving it once proves it for both.
struct Log<A: AggregateRules + Clone> {
    records: Vec<Record<A>>,
    snapshots: Vec<Snapshot<A>>,
}

impl<A: AggregateRules + Clone> Log<A> {
    fn new(genesis: A) -> Self {
        Self {
            records: Vec::new(),
            snapshots: vec![Snapshot {
                state: genesis,
                schema_version: 0,
                at: Seq(0),
                entropy_pos: DrawPos(0),
            }],
        }
    }

    fn record_at(&self, at: Seq) -> Result<&Record<A>, JournalError> {
        if at.0 == 0 || at.0 as usize > self.records.len() {
            return Err(JournalError::UnknownSeq { at });
        }
        Ok(&self.records[(at.0 - 1) as usize])
    }

    /// Append events and the entropy position as one record — the atomic unit the
    /// `Journal` contract requires (a real adapter does this in one transaction).
    fn append(&mut self, events: &[A::Event], entropy_pos: DrawPos) -> Result<Seq, JournalError> {
        self.records.push(Record {
            events: events.to_vec(),
            entropy_pos,
        });
        Ok(Seq(self.records.len() as u64))
    }

    fn entropy_pos(&self, at: Seq) -> Result<DrawPos, JournalError> {
        if at.0 == 0 {
            return Ok(self
                .snapshots
                .iter()
                .find(|s| s.at == Seq(0))
                .map_or(DrawPos(0), |s| s.entropy_pos));
        }
        Ok(self.record_at(at)?.entropy_pos)
    }

    fn head(&self) -> Option<Seq> {
        (!self.records.is_empty()).then_some(Seq(self.records.len() as u64))
    }

    fn events_since(&self, after: Option<Seq>) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        let start = after.map_or(0, |s| s.0 as usize);
        let type_name = Cow::Borrowed(core::any::type_name::<A::Event>());
        Ok(self
            .records
            .iter()
            .skip(start)
            .flat_map(|record| record.events.iter())
            .map(|event| VersionedEvent {
                event: event.clone(),
                type_name: type_name.clone(),
                version: 1,
            })
            .collect())
    }

    fn latest_snapshot(&self) -> Result<Option<Snapshot<A>>, JournalError> {
        Ok(self
            .snapshots
            .iter()
            .max_by_key(|s| s.at)
            .map(clone_snapshot))
    }

    // `snapshot` and `fork` round out the `Journal` surface the sync twin needs for
    // the contract; the async front end here never calls them.
    #[cfg(test)]
    fn snapshot(&mut self, snapshot: Snapshot<A>) -> Result<(), JournalError> {
        self.snapshots.push(snapshot);
        Ok(())
    }

    #[cfg(test)]
    fn fork(&self, at: Seq) -> Result<Self, JournalError> {
        if at.0 as usize > self.records.len() {
            return Err(JournalError::UnknownSeq { at });
        }
        let cutoff = at.0 as usize;
        Ok(Self {
            records: self
                .records
                .iter()
                .take(cutoff)
                .map(|r| Record {
                    events: r.events.clone(),
                    entropy_pos: r.entropy_pos,
                })
                .collect(),
            snapshots: self
                .snapshots
                .iter()
                .filter(|s| s.at <= at)
                .map(clone_snapshot)
                .collect(),
        })
    }
}

/// Clone a snapshot without requiring `Snapshot: Clone`.
fn clone_snapshot<A: AggregateRules + Clone>(snapshot: &Snapshot<A>) -> Snapshot<A> {
    Snapshot {
        state: snapshot.state.clone(),
        schema_version: snapshot.schema_version,
        at: snapshot.at,
        entropy_pos: snapshot.entropy_pos,
    }
}

// --- the async front end (what production uses) ----------------------------

/// An async, authoritative store over a [`Log`] — the stand-in for a postgres
/// adapter reached over an async client. Its operations are `async fn`, which is
/// exactly why it cannot implement the synchronous [`Journal`](ironstate_journal::Journal) trait.
struct AsyncStore<A: AggregateRules + Clone> {
    log: Log<A>,
    fail_next_append: bool,
}

impl<A: AggregateRules + Clone> AsyncStore<A> {
    fn connect(genesis: A) -> Self {
        Self {
            log: Log::new(genesis),
            fail_next_append: false,
        }
    }

    /// Arm the next `append` to fail — a stand-in for a dropped connection or a
    /// constraint violation mid-write, so the abort path can be exercised.
    fn arm_append_failure(&mut self) {
        self.fail_next_append = true;
    }

    async fn head(&self) -> Option<Seq> {
        rt::yield_now().await;
        self.log.head()
    }

    async fn entropy_pos(&self, at: Seq) -> Result<DrawPos, JournalError> {
        rt::yield_now().await;
        self.log.entropy_pos(at)
    }

    /// The entropy position recorded at the head, or `DrawPos(0)` if empty — the
    /// async equivalent of what `execute` reads before deciding.
    async fn head_pos(&self) -> Result<DrawPos, JournalError> {
        match self.head().await {
            Some(head) => self.entropy_pos(head).await,
            None => Ok(DrawPos(0)),
        }
    }

    async fn append(&mut self, events: &[A::Event], pos: DrawPos) -> Result<Seq, JournalError> {
        rt::yield_now().await;
        if self.fail_next_append {
            self.fail_next_append = false;
            return Err(JournalError::Storage(
                "simulated write failure (the row was not committed)".into(),
            ));
        }
        self.log.append(events, pos)
    }

    async fn events_since(
        &self,
        after: Option<Seq>,
    ) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        rt::yield_now().await;
        self.log.events_since(after)
    }

    async fn latest_snapshot(&self) -> Result<Option<Snapshot<A>>, JournalError> {
        rt::yield_now().await;
        self.log.latest_snapshot()
    }
}

/// The persistent loop against an async store — the recollect server loop. It owns
/// the IO (two `.await`s) and nothing else: `prepare`/`commit`/`abort` carry the
/// entropy-position capture, the append-before-evolve ordering, and the rewind, so
/// this can't drift from the built-in `execute`.
async fn execute_async<A>(
    store: &mut AsyncStore<A>,
    aggregate: &mut Aggregate<A>,
    cmd: &A::Command,
    ctx: &mut A::Ctx,
) -> Result<Seq, ExecuteError<A>>
where
    A: AggregateRules + Clone,
    A::Ctx: CtxEntropy,
{
    // 1. Read the head position from the store (async).
    let head = store.head_pos().await.map_err(ExecuteError::Journal)?;
    // 2. Decide and capture the position — pure, ironstate-owned.
    let prepared = prepare(aggregate, cmd, ctx, head).map_err(ExecuteError::Rejected)?;
    // 3. The one mutating await: append events + position atomically.
    let appended = store
        .append(prepared.events(), prepared.entropy_pos())
        .await;
    match appended {
        // 4a. Durable — evolve to match the log, then ack.
        Ok(seq) => {
            prepared.commit(aggregate);
            Ok(seq)
        }
        // 4b. Write failed — rewind the entropy stream; nothing observable changed.
        Err(error) => {
            prepared.abort(ctx);
            Err(ExecuteError::Journal(error))
        }
    }
}

/// Rebuild an aggregate from the async store — `resume`, async. The reads are
/// awaited; the rebuild is the pure [`replay`]. The resume position is the one
/// recorded **at the head**, not the snapshot's, which is the discipline the
/// contract enforces.
async fn resume_async<A>(
    store: &AsyncStore<A>,
    seed: &Seed,
) -> Result<(Aggregate<A>, SeededEntropy), ResumeError>
where
    A: AggregateRules + Clone,
{
    let snapshot = store
        .latest_snapshot()
        .await
        .map_err(ResumeError::Journal)?
        .ok_or(ResumeError::NoBase)?;
    let from = snapshot.at;
    let snapshot_pos = snapshot.entropy_pos;
    let events = store
        .events_since(Some(from))
        .await
        .map_err(ResumeError::Journal)?;

    let aggregate = replay(snapshot, &events).map_err(ResumeError::Restore)?;

    let resume_pos = match store.head().await {
        Some(head) => store
            .entropy_pos(head)
            .await
            .map_err(ResumeError::Journal)?,
        None => snapshot_pos,
    };
    Ok((aggregate, SeededEntropy::at(seed, resume_pos)))
}

// --- the sync twin (test-only: the contract's measuring stick) -------------

/// A synchronous [`Journal`](ironstate_journal::Journal) over the same [`Log`]. Production never constructs one;
/// it exists so `journal_contract_test!` can hold the storage to the seven-property
/// suite. The async front end inherits that proof because it drives the same `Log`.
#[cfg(test)]
struct SyncStore<A: AggregateRules + Clone>(Log<A>);

#[cfg(test)]
impl<A: AggregateRules + Clone> Journal<A> for SyncStore<A> {
    fn append(&mut self, events: &[A::Event], entropy_pos: DrawPos) -> Result<Seq, JournalError> {
        self.0.append(events, entropy_pos)
    }
    fn entropy_pos(&self, at: Seq) -> Result<DrawPos, JournalError> {
        self.0.entropy_pos(at)
    }
    fn head(&self) -> Option<Seq> {
        self.0.head()
    }
    fn events_since(&self, after: Option<Seq>) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        self.0.events_since(after)
    }
    fn snapshot(&mut self, snapshot: Snapshot<A>) -> Result<(), JournalError> {
        self.0.snapshot(snapshot)
    }
    fn latest_snapshot(&self) -> Result<Option<Snapshot<A>>, JournalError> {
        self.0.latest_snapshot()
    }
    fn fork(&self, at: Seq) -> Result<Self, JournalError> {
        Ok(SyncStore(self.0.fork(at)?))
    }
}

#[cfg(test)]
impl<A: AggregateRules + Clone> ContractJournal<A> for SyncStore<A> {
    fn fresh(genesis: A) -> Self {
        SyncStore(Log::new(genesis))
    }
}

// The headline: the durable storage passes the seven-property journal contract —
// the same bar `MemoryJournal` meets — even though production drives it async.
#[cfg(test)]
ironstate_journal::journal_contract_test!(SyncStore<Tally>, Tally);

// === a walk-through ========================================================

/// Build a context whose entropy is positioned at the store's head — what the loop
/// expects of a live stream.
async fn ctx_at_head(
    store: &AsyncStore<Tally>,
    seed: &Seed,
) -> Result<OwnedDeterministicCtx<u32>, JournalError> {
    let pos = store.head_pos().await?;
    Ok(OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(seed, pos)),
        actor: 0,
        now: LogicalTime(0),
    })
}

async fn demo() -> Result<()> {
    let seed = Seed([42u8; 32]);
    let mut store = AsyncStore::connect(genesis());
    let mut tally = Aggregate::new(genesis()).map_err(|e| anyhow!("{e}"))?;

    // Five rolls, each appended to the async store *before* it is applied.
    for _ in 0..5 {
        let mut ctx = ctx_at_head(&store, &seed)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        execute_async(&mut store, &mut tally, &Command::Roll, &mut ctx)
            .await
            .map_err(|e| anyhow!("{e}"))?;
    }
    println!(
        "after 5 rolls: head = {:?}, total = {}",
        store.head().await,
        tally.state().total
    );

    // Append-before-ack durability: arm a write failure, run a command, and confirm
    // nothing is observable — no row, no state change, no entropy consumed.
    let head_before = store.head().await;
    let total_before = tally.state().total;
    store.arm_append_failure();
    let mut ctx = ctx_at_head(&store, &seed)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let position_before = ctx.entropy.draws();
    let failed = execute_async(&mut store, &mut tally, &Command::Roll, &mut ctx).await;
    assert!(
        matches!(failed, Err(ExecuteError::Journal(_))),
        "the armed write should fail"
    );
    assert_eq!(store.head().await, head_before, "a failed write left a row");
    assert_eq!(
        tally.state().total,
        total_before,
        "a failed write changed state"
    );
    assert_eq!(
        ctx.entropy.draws(),
        position_before,
        "a failed write left entropy advanced"
    );
    println!("armed write failed; store, state, and entropy all untouched (abort rewound)");

    // The store is authoritative: a fresh process rebuilds the exact state by
    // resuming from it — entropy repositioned at the head, not an earlier snapshot.
    let (resumed, _entropy) = resume_async(&store, &seed)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    assert_eq!(
        resumed.state(),
        tally.state(),
        "resume reproduces the live state"
    );
    println!(
        "resumed from the async store; state matches: total = {}",
        resumed.state().total
    );

    Ok(())
}

fn main() -> Result<()> {
    rt::block_on(demo())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_runs() {
        rt::block_on(demo()).unwrap();
    }

    /// A failed async append leaves nothing: no row, no state change, and the
    /// entropy stream rewound to the head — the durability guarantee recollect needs.
    #[test]
    fn failed_async_append_leaves_nothing() {
        let seed = Seed([7u8; 32]);
        rt::block_on(async {
            let mut store = AsyncStore::connect(genesis());
            let mut tally = Aggregate::new(genesis()).unwrap();

            // One real append, so the head sits past genesis.
            let mut ctx = ctx_at_head(&store, &seed).await.unwrap();
            execute_async(&mut store, &mut tally, &Command::Roll, &mut ctx)
                .await
                .unwrap();

            let head_before = store.head().await;
            let total_before = tally.state().total;

            store.arm_append_failure();
            let mut ctx = ctx_at_head(&store, &seed).await.unwrap();
            let position_before = ctx.entropy.draws();
            let err = execute_async(&mut store, &mut tally, &Command::Roll, &mut ctx)
                .await
                .unwrap_err();

            assert!(matches!(err, ExecuteError::Journal(_)));
            assert_eq!(store.head().await, head_before);
            assert_eq!(tally.state().total, total_before);
            assert_eq!(ctx.entropy.draws(), position_before);
        });
    }

    /// Resuming from the async store reproduces a live run exactly — proof the
    /// entropy-position discipline held across the hand-rolled async loop.
    #[test]
    fn resume_matches_a_live_run() {
        let seed = Seed([9u8; 32]);
        rt::block_on(async {
            let mut store = AsyncStore::connect(genesis());
            let mut tally = Aggregate::new(genesis()).unwrap();
            for _ in 0..6 {
                let mut ctx = ctx_at_head(&store, &seed).await.unwrap();
                execute_async(&mut store, &mut tally, &Command::Roll, &mut ctx)
                    .await
                    .unwrap();
            }

            let (resumed, _entropy) = resume_async(&store, &seed).await.unwrap();
            assert_eq!(resumed.state(), tally.state());
        });
    }
}
