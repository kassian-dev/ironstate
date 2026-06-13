//! Redaction: the `Redact` derive builds a per-viewer view where the viewer
//! sees their own hidden values and everyone else sees only the residue. The
//! view type cannot even represent another principal's hidden value.
#![cfg(feature = "redaction")]

use ironstate_aggregate::{Conceal, Owned, OwnedView, PerPrincipal, Redact, View};

type ParticipantId = u32;

// A hidden hand; others learn only its size.
#[derive(Clone, Debug, PartialEq)]
struct Hand {
    cards: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
struct HandPublic {
    count: u8,
}
impl Conceal for Hand {
    type Concealed = HandPublic;
    fn conceal(&self) -> HandPublic {
        HandPublic {
            count: self.cards.len() as u8,
        }
    }
}

// A hidden deck; everyone (owner included) sees only its size.
#[derive(Clone, Debug, PartialEq)]
struct Deck {
    cards: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
struct DeckPublic {
    count: u8,
}
impl Conceal for Deck {
    type Concealed = DeckPublic;
    fn conceal(&self) -> DeckPublic {
        DeckPublic {
            count: self.cards.len() as u8,
        }
    }
}

// A hidden identity; others learn nothing at all.
#[derive(Clone, Debug, PartialEq)]
struct FabricationId(u32);
impl Conceal for FabricationId {
    type Concealed = ();
    fn conceal(&self) {}
}

#[derive(Redact, Clone, Debug)]
#[redact(principal = ParticipantId)]
struct MatchState {
    board: Vec<u8>,
    #[hidden]
    hands: PerPrincipal<ParticipantId, Hand>,
    #[hidden]
    fabrication: Owned<ParticipantId, FabricationId>,
    #[hidden(conceal)]
    decks: PerPrincipal<ParticipantId, Deck>,
    #[hidden(from = all)]
    #[allow(dead_code)]
    private_note: u64,
}

fn sample() -> MatchState {
    let mut hands = PerPrincipal::new();
    hands.insert(
        1,
        Hand {
            cards: vec![10, 11],
        },
    );
    hands.insert(
        2,
        Hand {
            cards: vec![20, 21, 22],
        },
    );

    let mut decks = PerPrincipal::new();
    decks.insert(
        1,
        Deck {
            cards: vec![1, 2, 3, 4],
        },
    );
    decks.insert(2, Deck { cards: vec![5, 6] });

    MatchState {
        board: vec![100, 101],
        hands,
        fabrication: Owned::new(1, FabricationId(7)),
        decks,
        private_note: 0xDEAD_BEEF,
    }
}

#[test]
fn public_fields_pass_through() {
    let view = sample().view_for(&1);
    assert_eq!(view.board, vec![100, 101]);
}

#[test]
fn owner_sees_their_hidden_value_others_see_residue() {
    let state = sample();
    let view = state.view_for(&1);

    // Participant 1 sees their own hand in full...
    assert_eq!(
        view.hands.mine,
        Some(Hand {
            cards: vec![10, 11]
        })
    );
    // ...and only the size of participant 2's hand.
    assert_eq!(view.hands.others.get(&2), Some(&HandPublic { count: 3 }));
    // The full hand of participant 2 is not representable in the view at all.
    assert!(!view.hands.others.contains_key(&1));
}

#[test]
fn owned_value_is_full_for_owner_concealed_for_others() {
    let state = sample();
    assert_eq!(
        state.view_for(&1).fabrication,
        OwnedView::Mine(FabricationId(7))
    );
    // A non-owner learns nothing (the residue is the unit type).
    assert_eq!(state.view_for(&2).fabrication, OwnedView::Concealed(()));
}

#[test]
fn conceal_fields_are_residue_for_everyone() {
    let state = sample();
    // Even the deck's owner sees only the count.
    let view = state.view_for(&1);
    assert_eq!(view.decks.get(&1), Some(&DeckPublic { count: 4 }));
    assert_eq!(view.decks.get(&2), Some(&DeckPublic { count: 2 }));
}

#[test]
fn views_of_two_principals_differ_only_in_their_own_hidden_data() {
    let state = sample();
    let view1 = state.view_for(&1);
    let view2 = state.view_for(&2);
    // Same public board, different owner-visible hands.
    assert_eq!(view1.board, view2.board);
    assert_ne!(view1.hands.mine, view2.hands.mine);
}
