//! Structural graph analysis behind the `analyze!` macro.
//!
//! Every claim it emits is labeled: `[proven]` for facts that hold by
//! construction over the variant-level graph, `[sampled]` for anything that
//! depends on the data a variant carries (which `test!` exercises instead).
//! There are no unlabeled claims.

use crate::kind;
use crate::machine::{EventKind, StateMachine};
use std::collections::BTreeSet;
use std::fmt;

/// How strongly a claim is backed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Holds by construction over the variant-level state graph.
    Proven,
    /// Observed at the variant level; data-dependent arms are left to `test!`.
    Sampled,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proven => f.write_str("proven"),
            Self::Sampled => f.write_str("sampled"),
        }
    }
}

/// One line of an analysis report.
#[derive(Debug, Clone)]
pub struct Claim {
    /// Whether this claim is proven or sampled.
    pub confidence: Confidence,
    /// True if this claim represents a design error that fails analysis.
    pub is_error: bool,
    /// The human-readable claim.
    pub message: String,
}

/// The result of analyzing a machine's state graph.
#[derive(Debug, Clone)]
pub struct Report {
    /// The analyzed machine's name.
    pub machine: &'static str,
    /// Every claim, in report order.
    pub claims: Vec<Claim>,
}

impl Report {
    /// Whether the report contains any design errors.
    pub fn has_errors(&self) -> bool {
        self.claims.iter().any(|c| c.is_error)
    }

    /// Panic with a teaching message if any design error was found.
    pub fn assert_ok(&self) {
        if !self.has_errors() {
            return;
        }
        let mut msg = format!("analysis of `{}` found design errors:\n", self.machine);
        for claim in self.claims.iter().filter(|c| c.is_error) {
            msg.push_str(&format!("  - [{}] {}\n", claim.confidence, claim.message));
        }
        msg.push_str(
            "Fix each error above: add the missing transition, make every state able to \
             reach a terminal, or remove the transition that structural enforcement blocks.",
        );
        panic!("{msg}");
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "→(Fe) ironstate analysis of `{}`", self.machine)?;
        for claim in &self.claims {
            let marker = if claim.is_error { "✗" } else { "·" };
            writeln!(f, "  {marker} {} [{}]", claim.message, claim.confidence)?;
        }
        Ok(())
    }
}

/// Analyze a machine's variant-level state graph.
pub fn analyze<S: StateMachine>() -> Report
where
    S::Event: EventKind + Clone,
{
    let states = S::state_variants();
    let events = S::Event::event_variants();
    let n = states.len();

    // Map each variant name to an index. Representatives are one per variant,
    // so names are the stable identity here.
    let names: Vec<&'static str> = states.iter().map(|s| s.variant_name()).collect();
    let index_of = |name: &str| names.iter().position(|n| *n == name);

    let terminal: Vec<bool> = states.iter().map(|s| s.is_terminal()).collect();

    // Edges over structurally-legal transitions, plus dead transitions that the
    // function defines but enforcement blocks.
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut dead: Vec<String> = Vec::new();
    let mut used_event: BTreeSet<&'static str> = BTreeSet::new();

    for (i, state) in states.iter().enumerate() {
        let restriction = state.restriction();
        for event in &events {
            let kind_ok = match restriction {
                None => true,
                Some(expected) => {
                    matches!(event.kinds(), Some(ek) if kind::intersects(expected, ek))
                }
            };
            let structurally_allowed = !terminal[i] && kind_ok;

            if let Some(target) = state.transition(event) {
                if structurally_allowed {
                    if let Some(j) = index_of(target.variant_name()) {
                        edges[i].push(j);
                        used_event.insert(event.variant_name());
                    }
                } else {
                    let why = if terminal[i] {
                        "the source state is terminal"
                    } else {
                        "the event's kind is not accepted by the source state"
                    };
                    dead.push(format!(
                        "transition {} --{}--> {} can never fire: {why}",
                        state.variant_name(),
                        event.variant_name(),
                        target.variant_name(),
                    ));
                }
            }
        }
    }

    let mut claims = Vec::new();

    // Reachability from the initial state.
    let initial = S::initial();
    let reachable = reachable_from(index_of(initial.variant_name()).unwrap_or(0), &edges, n);
    let unreachable: Vec<&str> = (0..n)
        .filter(|&i| !reachable[i])
        .map(|i| names[i])
        .collect();
    if unreachable.is_empty() {
        claims.push(proven(
            false,
            format!(
                "all {n} variants are reachable from {}",
                initial.variant_name()
            ),
        ));
    } else {
        for name in &unreachable {
            claims.push(proven(
                true,
                format!("state `{name}` is unreachable from the initial state"),
            ));
        }
    }

    // Deadlocks: a non-terminal state with no outbound transitions.
    for i in 0..n {
        if !terminal[i] && edges[i].is_empty() {
            claims.push(proven(
                true,
                format!(
                    "state `{}` is a deadlock: non-terminal with no outbound transitions",
                    names[i]
                ),
            ));
        }
    }

    // Can every state reach a terminal? An inescapable region is a design error.
    let can_reach_terminal = reaches_terminal(&edges, &terminal, n);
    for i in 0..n {
        if !terminal[i] && !can_reach_terminal[i] && !edges[i].is_empty() {
            claims.push(proven(
                true,
                format!(
                    "state `{}` can never reach a terminal state (inescapable cycle)",
                    names[i]
                ),
            ));
        }
    }

    // Dead transitions.
    if dead.is_empty() {
        claims.push(proven(false, "no dead transitions".to_string()));
    } else {
        for d in dead {
            claims.push(proven(true, d));
        }
    }

    // Unused events (informational, proven).
    let unused: Vec<&'static str> = events
        .iter()
        .map(|e| e.variant_name())
        .filter(|name| !used_event.contains(name))
        .collect();
    if !unused.is_empty() {
        claims.push(proven(
            false,
            format!("unused events (trigger no transition): {unused:?}"),
        ));
    }

    // Coverage — sampled, because data-carrying arms are collapsed to one variant.
    let total_pairs = n.saturating_mul(events.len());
    let producing: usize = edges.iter().map(|e| e.len()).sum();
    claims.push(Claim {
        confidence: Confidence::Sampled,
        is_error: false,
        message: format!(
            "coverage: {producing} of {total_pairs} (state, event) pairs produce transitions \
             — variant-level; data-dependent arms exercised by test!()"
        ),
    });

    Report {
        machine: states
            .first()
            .map(|_| std::any::type_name::<S>())
            .unwrap_or("<empty>"),
        claims,
    }
}

fn proven(is_error: bool, message: String) -> Claim {
    Claim {
        confidence: Confidence::Proven,
        is_error,
        message,
    }
}

fn reachable_from(start: usize, edges: &[Vec<usize>], n: usize) -> Vec<bool> {
    let mut seen = vec![false; n];
    if n == 0 {
        return seen;
    }
    let mut stack = vec![start];
    seen[start] = true;
    while let Some(i) = stack.pop() {
        for &j in &edges[i] {
            if !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    seen
}

fn reaches_terminal(edges: &[Vec<usize>], terminal: &[bool], n: usize) -> Vec<bool> {
    // Reverse-reachability from terminal states.
    let mut rev: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, outs) in edges.iter().enumerate() {
        for &j in outs {
            rev[j].push(i);
        }
    }
    let mut can = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        if terminal[i] {
            can[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        for &p in &rev[i] {
            if !can[p] {
                can[p] = true;
                stack.push(p);
            }
        }
    }
    can
}
