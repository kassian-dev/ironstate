//! What `Default` the derives actually need.
//!
//! Variant enumeration builds one representative per variant by constructing
//! the variant directly and filling each *field* with `Default::default()`. So
//! the requirement lands on the payload field types, never on the enum: these
//! types deliberately do **not** implement `Default`, and must still derive and
//! analyse.

use ironstate::prelude::*;

/// A payload that implements `Default` — the only thing that has to.
#[derive(Clone, Debug, Default, PartialEq)]
struct Reviewer(String);

/// A state machine with data-carrying variants whose **enum** is not `Default`.
#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Archived])]
enum Article {
    Draft,
    /// A named-field payload.
    Review {
        by: Reviewer,
        rounds: u32,
    },
    /// A tuple payload.
    Rejected(Reviewer),
    Archived,
}

/// An event enum with a data-carrying variant, likewise not `Default`.
#[derive(Event, Clone, Debug, PartialEq)]
enum Edit {
    Submit,
    Assign(Reviewer),
    Reject { reason: u32 },
    Archive,
}

impl TransitionRules for Article {
    type Event = Edit;
    fn transition(&self, event: &Edit) -> Option<Article> {
        match (self, event) {
            (Article::Draft, Edit::Submit) => Some(Article::Review {
                by: Reviewer::default(),
                rounds: 0,
            }),
            (Article::Review { by, .. }, Edit::Reject { .. }) => {
                Some(Article::Rejected(by.clone()))
            }
            (Article::Rejected(_), Edit::Archive) => Some(Article::Archived),
            _ => None,
        }
    }
}

// The whole point: this compiles, and `analyze!` walks every variant, without
// `Article` or `Edit` implementing `Default`.
#[cfg(test)]
ironstate::analyze!(Article);

#[test]
fn data_carrying_variants_need_default_only_on_their_payloads() {
    let mut machine = Machine::<Article>::new();
    assert_eq!(machine.state(), &Article::Draft);
    machine.apply(Edit::Submit).expect("Draft -> Review");
    assert!(matches!(machine.state(), Article::Review { .. }));
}
