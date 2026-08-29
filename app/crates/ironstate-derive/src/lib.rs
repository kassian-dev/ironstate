#![doc(
    html_logo_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/favicon-32.png"
)]
//! Derive macros for [`ironstate`](https://docs.rs/ironstate).
//!
//! - `#[derive(StateMachine)]` generates the structural metadata a machine
//!   needs: its initial state, terminal states, per-state event-kind
//!   restrictions, and (with `version`/`history`) versioned restore.
//! - `#[derive(Event)]` reads `#[event_kind]` and `#[likelihood]` and generates
//!   the event metadata the runtime and verification macros consume.
#![warn(missing_docs)]

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod common;
mod event;
mod statemachine;

/// Derive `StateMachine` for an enum of states.
///
/// ```ignore
/// #[derive(StateMachine, Clone, Debug, PartialEq)]
/// #[state_machine(initial = Draft, terminal = [Published, Archived])]
/// enum Article { Draft, Review, Published, Archived }
/// ```
///
/// # Data-carrying fields must implement `Default`
///
/// `analyze!` and `test!` walk every state variant, and the derive builds one
/// representative per variant to walk. Analysis is variant-level, so any
/// data-carrying fields are filled with `Default::default()` — the payload types
/// must therefore implement `Default`, or you can hand-write the `StateMachine` impl.
/// Fieldless enums (including the aggregate tier's phase machines) need nothing
/// extra.
#[proc_macro_derive(StateMachine, attributes(state_machine, only_accepts))]
pub fn derive_state_machine(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    statemachine::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `Event` for an enum of events.
///
/// ```ignore
/// #[derive(Event, Clone, Debug, PartialEq)]
/// enum Edit {
///     Submit,
///     #[event_kind = "operator"]
///     Approve,
///     #[likelihood = "rare"]
///     Reject,
/// }
/// ```
///
/// # Data-carrying variants must implement `Default`
///
/// As with `StateMachine`, variant enumeration builds one representative per
/// variant and fills its fields with `Default::default()`, so a data-carrying
/// event variant must implement `Default`.
#[proc_macro_derive(Event, attributes(event_kind, likelihood))]
pub fn derive_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    event::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
