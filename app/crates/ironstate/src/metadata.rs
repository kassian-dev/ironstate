//! Structured, variant-level description of a machine's state graph.

use crate::kind;
use crate::machine::{EventKind, StateMachine};

/// A static view of a machine's structure, built from the derived metadata.
///
/// Transitions listed here are only those that survive structural enforcement:
/// a transition the transition function defines but that a terminal state or an
/// event-kind restriction would block does not appear (it is a *dead*
/// transition, surfaced by `analyze!`).
pub struct MachineMetadata<S: StateMachine>
where
    S::Event: EventKind,
{
    /// The declared initial state.
    pub initial_state: S,
    /// Every state variant (one representative each).
    pub all_states: Vec<S>,
    /// The terminal state variants.
    pub terminal_states: Vec<S>,
    /// Legal `(from, event, to)` transitions, variant-level.
    pub transitions: Vec<(S, S::Event, S)>,
}

/// Build the metadata for a machine by walking every state and event variant.
pub(crate) fn build<S: StateMachine>() -> MachineMetadata<S>
where
    S::Event: EventKind + Clone,
{
    let all_states = S::state_variants();
    let terminal_states = all_states
        .iter()
        .filter(|s| s.is_terminal())
        .cloned()
        .collect();
    let events = S::Event::event_variants();

    let mut transitions = Vec::new();
    for state in &all_states {
        if state.is_terminal() {
            continue;
        }
        let restriction = state.restriction();
        for event in &events {
            if let Some(expected) = restriction {
                let accepted = matches!(event.kinds(), Some(ek) if kind::intersects(expected, ek));
                if !accepted {
                    continue;
                }
            }
            if let Some(target) = state.transition(event) {
                transitions.push((state.clone(), event.clone(), target));
            }
        }
    }

    MachineMetadata {
        initial_state: S::initial(),
        all_states,
        terminal_states,
        transitions,
    }
}
