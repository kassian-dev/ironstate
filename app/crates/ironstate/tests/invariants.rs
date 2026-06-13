//! Declared invariants are checked by `test!`, and a machine without them still
//! runs. Also pins the autoref-specialization probe `test!` uses to find them.

use core::marker::PhantomData;
use ironstate::prelude::*;

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Empty, terminal = [Closed])]
enum Cart {
    Empty,
    Active,
    Closed,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum CartEvent {
    Add,
    Checkout,
}

impl TransitionRules for Cart {
    type Event = CartEvent;
    fn transition(&self, event: &CartEvent) -> Option<Cart> {
        use Cart::*;
        use CartEvent::*;
        match (self, event) {
            (Empty, Add) => Some(Active),
            (Active, Add) => Some(Active),
            (Empty, Checkout) => Some(Closed),
            (Active, Checkout) => Some(Closed),
            _ => None,
        }
    }
}

impl Invariants for Cart {
    fn invariants() -> Vec<Invariant<Self, Self::Event>> {
        vec![
            Invariant::custom("closed carts stay closed").assert(|before, _event, after| {
                if before == &Cart::Closed {
                    after.is_none()
                } else {
                    true
                }
            }),
        ]
    }
}

ironstate::analyze!(Cart);
ironstate::test!(Cart, cases = 300, max_steps = 40);

#[test]
fn probe_picks_up_declared_invariants() {
    // Both traits must be in scope for the autoref trick; `Cart` has invariants
    // so resolution lands on `ViaImpl` and the `ViaNone` import reads as unused.
    #[allow(unused_imports)]
    use ironstate::testing_support::probe::{Probe, ViaImpl as _, ViaNone as _};
    let probe = Probe::<Cart>(PhantomData);
    let invariants = (&&probe).collect();
    assert_eq!(invariants.len(), 1);
    assert_eq!(invariants[0].description(), "closed carts stay closed");
}
