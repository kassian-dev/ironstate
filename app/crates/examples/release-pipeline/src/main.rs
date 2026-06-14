#![doc = include_str!("../README.md")]

use ironstate::prelude::*;

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Failed, RolledBack, Retired])]
pub enum Release {
    Draft,
    Building,
    Testing,
    /// Waiting on a human; only operator events are accepted.
    #[only_accepts(kind = "operator")]
    AwaitingApproval,
    /// Controlled by the deploy target; only its signals are accepted.
    #[only_accepts(kind = "external")]
    Deploying,
    Live,
    Failed,
    RolledBack,
    Retired,
}

#[derive(Event, Clone, Debug, PartialEq)]
pub enum Signal {
    Submit,
    BuildOk,
    BuildFailed,
    TestsPassed,
    TestsFailed,
    #[event_kind = "operator"]
    Approve,
    #[event_kind = "operator"]
    Reject,
    #[event_kind = "external"]
    DeploySucceeded,
    #[event_kind = "external"]
    DeployFailed,
    Rollback,
    Retire,
}

impl TransitionRules for Release {
    type Event = Signal;
    fn transition(&self, signal: &Signal) -> Option<Release> {
        use Release::*;
        use Signal::*;
        match (self, signal) {
            (Draft, Submit) => Some(Building),
            (Building, BuildOk) => Some(Testing),
            (Building, BuildFailed) => Some(Failed),
            (Testing, TestsPassed) => Some(AwaitingApproval),
            (Testing, TestsFailed) => Some(Failed),
            (AwaitingApproval, Approve) => Some(Deploying),
            (AwaitingApproval, Reject) => Some(Failed),
            (Deploying, DeploySucceeded) => Some(Live),
            (Deploying, DeployFailed) => Some(RolledBack),
            (Live, Rollback) => Some(RolledBack),
            (Live, Retire) => Some(Retired),
            _ => None,
        }
    }
}

impl Invariants for Release {
    fn invariants() -> Vec<Invariant<Self, Self::Event>> {
        vec![
            // A release can only go Live straight out of Deploying — never
            // skipping the build/test/approve gates.
            Invariant::custom("Live is only reached from Deploying").assert(
                |before, _event, after| {
                    !matches!(after, Some(Release::Live)) || before == &Release::Deploying
                },
            ),
        ]
    }
}

// Verification: graph analysis (proves reachability, no deadlocks/dead
// transitions) and randomized property testing (checks the invariant).
ironstate::analyze!(Release);
ironstate::test!(Release, cases = 500, max_steps = 30);

fn main() {
    let mut release = Machine::<Release>::new();
    for signal in [
        Signal::Submit,
        Signal::BuildOk,
        Signal::TestsPassed,
        Signal::Approve,
        Signal::DeploySucceeded,
        Signal::Retire,
    ] {
        match release.apply(signal.clone()) {
            Ok(state) => println!("{signal:?} -> {state:?}"),
            Err(error) => println!("{signal:?} rejected: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_reaches_retired() {
        let mut release = Machine::<Release>::new();
        assert_eq!(release.apply(Signal::Submit).unwrap(), Release::Building);
        assert_eq!(release.apply(Signal::BuildOk).unwrap(), Release::Testing);
        assert_eq!(
            release.apply(Signal::TestsPassed).unwrap(),
            Release::AwaitingApproval
        );
        assert_eq!(release.apply(Signal::Approve).unwrap(), Release::Deploying);
        assert_eq!(
            release.apply(Signal::DeploySucceeded).unwrap(),
            Release::Live
        );
        assert_eq!(release.apply(Signal::Retire).unwrap(), Release::Retired);
        // Retired is terminal.
        assert!(matches!(
            release.apply(Signal::Rollback).unwrap_err(),
            TransitionError::TerminalState { .. }
        ));
    }

    #[test]
    fn approval_gate_rejects_non_operator_events() {
        let mut release = Machine::restore(Release::AwaitingApproval);
        // A default-kind signal is rejected on the kind check, before the
        // transition function runs.
        let err = release.apply(Signal::Submit).unwrap_err();
        assert!(matches!(err, TransitionError::EventKindRejected { .. }));
        // An external signal is also wrong here — only operator events pass.
        assert!(matches!(
            release.apply(Signal::DeploySucceeded).unwrap_err(),
            TransitionError::EventKindRejected { .. }
        ));
        // The operator approval is accepted.
        assert_eq!(release.apply(Signal::Approve).unwrap(), Release::Deploying);
    }

    #[test]
    fn a_failed_build_is_terminal() {
        let mut release = Machine::<Release>::new();
        release.apply(Signal::Submit).unwrap();
        assert_eq!(release.apply(Signal::BuildFailed).unwrap(), Release::Failed);
        assert!(!release.could_apply(&Signal::Submit));
    }
}
