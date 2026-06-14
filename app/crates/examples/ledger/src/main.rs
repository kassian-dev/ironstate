#![doc = include_str!("../README.md")]

use anyhow::Result;
use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateInvariant, AggregateInvariants, AggregateRules,
    DrawPos, EntropySource, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy, StableHash,
};
use ironstate_journal::{Journal, MemoryJournal, execute, resume};
use proptest::prelude::*;

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Open, terminal = [Closed])]
enum AccountPhase {
    Open,
    Frozen,
    Closed,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum PhaseStep {
    Freeze,
    Unfreeze,
    Close,
}

impl TransitionRules for AccountPhase {
    type Event = PhaseStep;
    fn transition(&self, step: &PhaseStep) -> Option<AccountPhase> {
        use AccountPhase::*;
        use PhaseStep::*;
        match (self, step) {
            (Open, Freeze) => Some(Frozen),
            (Frozen, Unfreeze) => Some(Open),
            (Open, Close) | (Frozen, Close) => Some(Closed),
            _ => None,
        }
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Deposit { cents: u64 },
    Withdraw { cents: u64 },
    Freeze,
    Unfreeze,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
enum LedgerEvent {
    Deposited { cents: u64 },
    Withdrawn { cents: u64 },
    Frozen,
    Unfrozen,
    Closed,
}

#[derive(Debug, thiserror::Error)]
enum LedgerError {
    #[error("the account is closed")]
    AccountClosed,
    #[error("the account is frozen")]
    AccountFrozen,
    #[error("insufficient funds")]
    Overdraft,
    #[error("the account must be empty to close")]
    MustBeEmpty,
    #[error("invalid for the account's current phase")]
    InvalidTransition,
}

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Account {
    phase: AccountPhase,
    balance_cents: u64,
}

impl Account {
    fn open() -> Self {
        Self {
            phase: AccountPhase::Open,
            balance_cents: 0,
        }
    }
}

impl AggregateRules for Account {
    type Phase = AccountPhase;
    type Command = Command;
    type Event = LedgerEvent;
    type Error = LedgerError;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> AccountPhase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<LedgerEvent>, LedgerError> {
        use AccountPhase::*;
        if self.phase == Closed {
            return Err(LedgerError::AccountClosed);
        }
        match cmd {
            Command::Deposit { cents } => {
                if self.phase == Frozen {
                    return Err(LedgerError::AccountFrozen);
                }
                Ok(vec![LedgerEvent::Deposited { cents: *cents }])
            }
            Command::Withdraw { cents } => {
                if self.phase == Frozen {
                    return Err(LedgerError::AccountFrozen);
                }
                if self.balance_cents < *cents {
                    return Err(LedgerError::Overdraft);
                }
                Ok(vec![LedgerEvent::Withdrawn { cents: *cents }])
            }
            Command::Freeze => {
                if self.phase == Open {
                    Ok(vec![LedgerEvent::Frozen])
                } else {
                    Err(LedgerError::InvalidTransition)
                }
            }
            Command::Unfreeze => {
                if self.phase == Frozen {
                    Ok(vec![LedgerEvent::Unfrozen])
                } else {
                    Err(LedgerError::InvalidTransition)
                }
            }
            Command::Close => {
                if self.balance_cents != 0 {
                    return Err(LedgerError::MustBeEmpty);
                }
                Ok(vec![LedgerEvent::Closed])
            }
        }
    }

    fn evolve(&mut self, event: &LedgerEvent) {
        match event {
            LedgerEvent::Deposited { cents } => self.balance_cents += *cents,
            LedgerEvent::Withdrawn { cents } => self.balance_cents -= *cents,
            LedgerEvent::Frozen => self.phase = AccountPhase::Frozen,
            LedgerEvent::Unfrozen => self.phase = AccountPhase::Open,
            LedgerEvent::Closed => self.phase = AccountPhase::Closed,
        }
    }
}

impl AggregateInvariants for Account {
    fn invariants() -> Vec<AggregateInvariant<Self>> {
        vec![
            // A withdrawal is only ever applied when the funds are there, so the
            // balance can never be driven negative (it never underflows).
            AggregateInvariant::<Self>::custom("withdrawals never overdraw").assert(
                |before, event, _after| match event {
                    LedgerEvent::Withdrawn { cents } => before.balance_cents >= *cents,
                    _ => true,
                },
            ),
        ]
    }
}

impl AggregateArbitrary for Account {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(Account::open()).boxed()
    }

    fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
        // Phase-agnostic: decide rejects commands invalid for the current phase,
        // and the test driver records and skips those rejections.
        prop_oneof![
            3 => (1u64..1000).prop_map(|cents| Command::Deposit { cents }),
            3 => (1u64..1000).prop_map(|cents| Command::Withdraw { cents }),
            1 => Just(Command::Freeze),
            1 => Just(Command::Unfreeze),
            1 => Just(Command::Close),
        ]
        .boxed()
    }

    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        OwnedDeterministicCtx {
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

// The full verification ladder for the aggregate tier.
ironstate_aggregate::test!(Account, cases = 300, max_steps = 40);
ironstate_aggregate::determinism_test!(Account, cases = 100, max_steps = 40);
ironstate_journal::journal_contract_test!(Account);

fn ctx_at_head(journal: &MemoryJournal<Account>) -> OwnedDeterministicCtx<u32> {
    let pos = journal
        .head()
        .map_or(DrawPos(0), |h| journal.entropy_pos(h).unwrap());
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(&Seed([0; 32]), pos)),
        actor: 0,
        now: LogicalTime(0),
    }
}

fn run_demo() -> Result<()> {
    let mut journal = MemoryJournal::new(Account::open());
    let mut account = Aggregate::new(Account::open())?;

    for command in [
        Command::Deposit { cents: 10_000 },
        Command::Withdraw { cents: 3_500 },
    ] {
        let mut ctx = ctx_at_head(&journal);
        execute(&mut journal, &mut account, &command, &mut ctx)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    println!(
        "balance after deposit + withdrawal: {} cents",
        account.state().balance_cents
    );

    // Resume rebuilds the balance from the event log alone.
    let seed = Seed([0; 32]);
    let (resumed, _entropy) =
        resume::<Account, _>(&journal, &seed).map_err(|e| anyhow::anyhow!("{e}"))?;
    assert_eq!(resumed.state().balance_cents, account.state().balance_cents);
    println!(
        "resumed balance matches: {} cents",
        resumed.state().balance_cents
    );

    // An overdraft is rejected and changes nothing.
    let mut ctx = ctx_at_head(&journal);
    let before = account.state().balance_cents;
    let rejected = execute(
        &mut journal,
        &mut account,
        &Command::Withdraw { cents: 1_000_000 },
        &mut ctx,
    );
    assert!(rejected.is_err());
    assert_eq!(account.state().balance_cents, before);
    println!("overdraft rejected; balance unchanged at {before} cents");

    Ok(())
}

fn main() -> Result<()> {
    run_demo()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironstate_aggregate::Rejection;

    #[test]
    fn deposit_withdraw_resume() {
        run_demo().unwrap();
    }

    #[test]
    fn overdraft_is_rejected() {
        let mut account = Aggregate::new(Account::open()).unwrap();
        let mut ctx = OwnedDeterministicCtx {
            entropy: Box::new(SeededEntropy::from_seed(&Seed([0; 32]))),
            actor: 0,
            now: LogicalTime(0),
        };
        account
            .handle(&Command::Deposit { cents: 50 }, &mut ctx)
            .unwrap();
        let err = account
            .handle(&Command::Withdraw { cents: 100 }, &mut ctx)
            .unwrap_err();
        assert!(matches!(err, Rejection::Domain(LedgerError::Overdraft)));
        assert_eq!(account.state().balance_cents, 50);
    }

    #[test]
    fn a_frozen_account_blocks_deposits() {
        let mut account = Aggregate::new(Account::open()).unwrap();
        let mut ctx = OwnedDeterministicCtx {
            entropy: Box::new(SeededEntropy::from_seed(&Seed([0; 32]))),
            actor: 0,
            now: LogicalTime(0),
        };
        account.handle(&Command::Freeze, &mut ctx).unwrap();
        let err = account
            .handle(&Command::Deposit { cents: 10 }, &mut ctx)
            .unwrap_err();
        assert!(matches!(err, Rejection::Domain(LedgerError::AccountFrozen)));
    }
}
