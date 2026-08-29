#![doc = include_str!("../README.md")]

use anyhow::{Result, anyhow};
use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, DrawPos, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy,
};
use ironstate_journal::{
    Journal, JournalError, Pending, Seq, Snapshot, StreamId, VersionedEvent, execute_in,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

// === the domain ===========================================================

/// Where an order is in its life. Shipping is the transition that causes work
/// outside the aggregate, so it is the one this example is about.
#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Placed, terminal = [Delivered, Cancelled])]
enum Phase {
    Placed,
    Shipped,
    Delivered,
    Cancelled,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Step {
    Ship,
    Deliver,
    Cancel,
}

impl TransitionRules for Phase {
    type Event = Step;
    fn transition(&self, step: &Step) -> Option<Phase> {
        match (self, step) {
            (Phase::Placed, Step::Ship) => Some(Phase::Shipped),
            (Phase::Placed, Step::Cancel) => Some(Phase::Cancelled),
            (Phase::Shipped, Step::Deliver) => Some(Phase::Delivered),
            _ => None,
        }
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Ship { carrier: String },
    Deliver,
}

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Shipped { carrier: String },
    Delivered,
}

#[derive(Debug, thiserror::Error)]
enum OrderError {
    #[error(
        "an order can only ship while it is placed.\n\
         This one is {phase:?}.\n\
         Check the phase before issuing Ship, or use `why_not` to ask first."
    )]
    NotShippable { phase: Phase },
}

#[derive(Clone, Debug, PartialEq)]
struct Order {
    phase: Phase,
    carrier: Option<String>,
}

impl Order {
    fn placed() -> Self {
        Self {
            phase: Phase::Placed,
            carrier: None,
        }
    }
}

impl AggregateRules for Order {
    type Phase = Phase;
    type Command = Command;
    type Event = Event;
    type Error = OrderError;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<Event>, OrderError> {
        match cmd {
            Command::Ship { carrier } => {
                if self.phase != Phase::Placed {
                    return Err(OrderError::NotShippable {
                        phase: self.phase.clone(),
                    });
                }
                Ok(vec![Event::Shipped {
                    carrier: carrier.clone(),
                }])
            }
            Command::Deliver => Ok(vec![Event::Delivered]),
        }
    }

    fn evolve(&mut self, event: &Event) {
        match event {
            Event::Shipped { carrier } => {
                self.phase = Phase::Shipped;
                self.carrier = Some(carrier.clone());
            }
            Event::Delivered => self.phase = Phase::Delivered,
        }
    }
}

// === the store: a stand-in for SQLite or Postgres =========================
//
// Three tables and a transaction over all of them. A real adapter replaces the
// maps with rows and `commit` with `COMMIT`; nothing about the journal
// integration changes.

/// One appended record: a batch of events and the entropy position they
/// consumed, stored as one unit because replay cannot recompute the position.
#[derive(Clone)]
struct Record {
    events: Vec<Event>,
    entropy_pos: DrawPos,
}

/// The read model a customer-facing screen would query. It exists whether or
/// not there is a journal, which is exactly why it must commit with one.
#[derive(Clone, Debug, PartialEq)]
struct OrderRow {
    status: &'static str,
    carrier: Option<String>,
}

/// Work the transition causes that happens outside this process.
#[derive(Clone, Debug, PartialEq)]
enum Job {
    ShipmentConfirmation { order: String, carrier: String },
}

/// The caller's unit of work. Every write — the journal's and the caller's —
/// is staged here and becomes visible only in [`Store::commit`]. Dropping it
/// instead is the rollback, and needs no code.
#[derive(Default)]
struct Transaction {
    appends: Vec<(StreamId, Record)>,
    read_model: Vec<(String, OrderRow)>,
    outbox: Vec<Job>,
}

impl Transaction {
    /// Stage a read-model write. A real adapter runs an `UPDATE` on `tx`.
    fn upsert_read_model(&mut self, order: &str, row: OrderRow) {
        self.read_model.push((order.to_owned(), row));
    }

    /// Stage an outbound job. A real adapter `INSERT`s into an outbox table
    /// that a worker polls — which is the whole reason this must be one
    /// transaction rather than a queue publish.
    fn enqueue(&mut self, job: Job) {
        self.outbox.push(job);
    }
}

/// The durable side. Holds the journal's records alongside the tables the
/// application would have anyway.
#[derive(Default)]
struct Store {
    records: BTreeMap<StreamId, Vec<Record>>,
    read_model: BTreeMap<String, OrderRow>,
    outbox: Vec<Job>,
}

impl Store {
    fn begin(&self) -> Transaction {
        Transaction::default()
    }

    /// Apply everything the transaction staged, in one step. Anything not
    /// committed this way never becomes visible to anyone.
    fn commit(&mut self, tx: Transaction) {
        for (stream, record) in tx.appends {
            self.records.entry(stream).or_default().push(record);
        }
        for (order, row) in tx.read_model {
            self.read_model.insert(order, row);
        }
        self.outbox.extend(tx.outbox);
    }

    fn records(&self, stream: &StreamId) -> &[Record] {
        self.records.get(stream).map_or(&[], Vec::as_slice)
    }
}

impl Journal<Order> for Store {
    /// The caller's transaction, not one the journal invents.
    type Tx<'a> = Transaction;

    fn append_in(
        &mut self,
        tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        events: &[Event],
        entropy_pos: DrawPos,
    ) -> Result<Seq, JournalError> {
        tx.appends.push((
            stream.clone(),
            Record {
                events: events.to_vec(),
                entropy_pos,
            },
        ));
        // The sequence this append *will* occupy once the transaction commits:
        // what is durable already, plus what this transaction has staged for
        // the same stream.
        let staged = tx.appends.iter().filter(|(s, _)| s == stream).count();
        Ok(Seq((self.records(stream).len() + staged) as u64))
    }

    fn snapshot_in(
        &mut self,
        _tx: &mut Self::Tx<'_>,
        _stream: &StreamId,
        _snapshot: Snapshot<Order>,
    ) -> Result<(), JournalError> {
        // Snapshots are not part of what this example shows; a real adapter
        // stages them into `tx` exactly as `append_in` does.
        Ok(())
    }

    fn entropy_pos(&self, stream: &StreamId, at: Seq) -> Result<DrawPos, JournalError> {
        if at.0 == 0 {
            return Ok(DrawPos(0));
        }
        let records = self.records(stream);
        if at.0 > records.len() as u64 {
            return Err(JournalError::UnknownSeq { at });
        }
        Ok(records[(at.0 - 1) as usize].entropy_pos)
    }

    fn head(&self, stream: &StreamId) -> Option<Seq> {
        let records = self.records(stream);
        (!records.is_empty()).then_some(Seq(records.len() as u64))
    }

    fn events_since(
        &self,
        stream: &StreamId,
        after: Option<Seq>,
    ) -> Result<Vec<VersionedEvent<Order>>, JournalError> {
        let start = after.map_or(0, |s| usize::try_from(s.0).unwrap_or(usize::MAX));
        let type_name = Cow::Borrowed(core::any::type_name::<Event>());
        Ok(self
            .records(stream)
            .iter()
            .enumerate()
            .skip(start)
            .flat_map(|(i, record)| {
                // Every event carries the Seq of the record it came from, not
                // its index in this flattened list.
                let seq = Seq(i as u64 + 1);
                record.events.iter().map(move |event| (seq, event))
            })
            .map(|(seq, event)| VersionedEvent::new(event.clone(), seq, type_name.clone(), 1))
            .collect())
    }

    fn latest_snapshot(&self, _stream: &StreamId) -> Result<Option<Snapshot<Order>>, JournalError> {
        Ok(Some(Snapshot {
            state: Order::placed(),
            schema_version: 0,
            at: Seq(0),
            entropy_pos: DrawPos(0),
        }))
    }
}

// === the persistent loop ==================================================

fn ctx(seed: &Seed) -> OwnedDeterministicCtx<u32> {
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(seed, DrawPos(0))),
        actor: 1,
        now: LogicalTime(0),
    }
}

/// Ship an order: append the event, project the read model, and enqueue the
/// confirmation — all in one transaction.
///
/// `commit` decides whether any of it happened. Returning the `Pending`
/// undriven would be a bug, so this drives it before returning.
fn ship(
    store: &mut Store,
    stream: &StreamId,
    order: &mut Aggregate<Order>,
    id: &str,
    carrier: &str,
    ctx: &mut OwnedDeterministicCtx<u32>,
    fail_the_transaction: bool,
) -> Result<Option<Seq>> {
    let cmd = Command::Ship {
        carrier: carrier.to_owned(),
    };

    let mut tx = store.begin();
    let pending: Pending<Order> = execute_in(store, &mut tx, stream, order, &cmd, ctx)
        .map_err(|e| anyhow!("ship rejected: {e}"))?;

    // The caller's own writes, into the same transaction the append joined.
    tx.upsert_read_model(
        id,
        OrderRow {
            status: "shipped",
            carrier: Some(carrier.to_owned()),
        },
    );
    tx.enqueue(Job::ShipmentConfirmation {
        order: id.to_owned(),
        carrier: carrier.to_owned(),
    });

    if fail_the_transaction {
        // A constraint violation, a lost connection, a deliberate rollback:
        // dropping the transaction discards every staged write at once.
        drop(tx);
        pending.abort(ctx);
        return Ok(None);
    }

    store.commit(tx);
    Ok(Some(pending.commit(order)))
}

fn describe(store: &Store, stream: &StreamId, order: &Aggregate<Order>) -> String {
    format!(
        "events={} read_model={:?} outbox={} aggregate={:?}",
        store.records(stream).len(),
        store.read_model.get("order-1").map(|r| r.status),
        store.outbox.len(),
        order.state().phase,
    )
}

fn main() -> Result<()> {
    let seed = Seed([7; 32]);
    let stream = StreamId::new("order-1");
    let mut store = Store::default();
    let mut order = Aggregate::new(Order::placed()).map_err(|e| anyhow!("{e}"))?;
    let mut context = ctx(&seed);

    println!("before          {}", describe(&store, &stream, &order));

    // A transaction that fails: nothing may survive it.
    let rolled_back = ship(
        &mut store,
        &stream,
        &mut order,
        "order-1",
        "Royal Mail",
        &mut context,
        true,
    )?;
    assert!(rolled_back.is_none());
    println!("after rollback  {}", describe(&store, &stream, &order));

    // The same command, committed.
    let seq = ship(
        &mut store,
        &stream,
        &mut order,
        "order-1",
        "Royal Mail",
        &mut context,
        false,
    )?
    .ok_or_else(|| anyhow!("the commit path returned no sequence"))?;
    println!("after commit    {}", describe(&store, &stream, &order));
    println!("\nappended at {seq:?}; the confirmation is queued because the append committed");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        Store,
        StreamId,
        Aggregate<Order>,
        OwnedDeterministicCtx<u32>,
    ) {
        let seed = Seed([7; 32]);
        (
            Store::default(),
            StreamId::new("order-1"),
            Aggregate::new(Order::placed()).expect("placed is the initial phase"),
            ctx(&seed),
        )
    }

    /// The whole point: a failed transaction leaves no event, no projection, no
    /// queued job, and an un-evolved aggregate. Any one of those surviving is
    /// the split-brain the outbox pattern exists to prevent.
    #[test]
    fn a_rolled_back_transaction_leaves_nothing_behind() {
        let (mut store, stream, mut order, mut context) = fixture();

        let result = ship(
            &mut store,
            &stream,
            &mut order,
            "order-1",
            "Royal Mail",
            &mut context,
            true,
        )
        .expect("the command itself is valid");

        assert!(result.is_none());
        assert_eq!(store.records(&stream).len(), 0, "no event was appended");
        assert!(store.read_model.is_empty(), "no projection was written");
        assert!(store.outbox.is_empty(), "no job was queued");
        assert_eq!(
            order.state().phase,
            Phase::Placed,
            "the aggregate must not have advanced past the durable log",
        );
    }

    /// And the mirror: on commit all four move together.
    #[test]
    fn a_committed_transaction_lands_all_of_it() {
        let (mut store, stream, mut order, mut context) = fixture();

        let seq = ship(
            &mut store,
            &stream,
            &mut order,
            "order-1",
            "Royal Mail",
            &mut context,
            false,
        )
        .expect("valid")
        .expect("committed");

        assert_eq!(seq, Seq(1));
        assert_eq!(store.records(&stream).len(), 1);
        assert_eq!(
            store.read_model.get("order-1").map(|r| r.status),
            Some("shipped"),
        );
        assert_eq!(
            store.outbox,
            vec![Job::ShipmentConfirmation {
                order: "order-1".to_owned(),
                carrier: "Royal Mail".to_owned(),
            }],
        );
        assert_eq!(order.state().phase, Phase::Shipped);
    }

    /// A rollback must leave the *next* attempt able to succeed — the entropy
    /// stream is rewound and the sequence is not consumed, so the retry lands
    /// at Seq(1) exactly as a first attempt would.
    #[test]
    fn a_retry_after_rollback_behaves_like_a_first_attempt() {
        let (mut store, stream, mut order, mut context) = fixture();

        ship(
            &mut store,
            &stream,
            &mut order,
            "order-1",
            "Royal Mail",
            &mut context,
            true,
        )
        .expect("valid");

        let seq = ship(
            &mut store,
            &stream,
            &mut order,
            "order-1",
            "Royal Mail",
            &mut context,
            false,
        )
        .expect("valid")
        .expect("committed");

        assert_eq!(seq, Seq(1), "the rolled-back attempt consumed no sequence");
        assert_eq!(store.records(&stream).len(), 1);
        assert_eq!(store.outbox.len(), 1, "the job is queued exactly once");
    }

    /// Staged writes are invisible until commit — including to the journal's
    /// own reads, which is why `execute_in` is one call per transaction.
    #[test]
    fn staged_writes_are_invisible_until_commit() {
        let (mut store, stream, order, mut context) = fixture();

        let mut tx = store.begin();
        let cmd = Command::Ship {
            carrier: "Royal Mail".to_owned(),
        };
        let pending = execute_in(&mut store, &mut tx, &stream, &order, &cmd, &mut context)
            .expect("append staged");

        assert_eq!(
            store.head(&stream),
            None,
            "a staged append is not visible to the journal's own reads",
        );

        store.commit(tx);
        let mut order = order;
        pending.commit(&mut order);
        assert_eq!(store.head(&stream), Some(Seq(1)));
    }
}
