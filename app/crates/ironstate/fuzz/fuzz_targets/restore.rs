#![no_main]
//! Fuzz the one place untrusted bytes re-enter ironstate: versioned restore.
//!
//! `restore_versioned` parses a `{version, payload}` envelope, dispatches on the
//! version, decodes the payload, and walks the `MigrateFrom` chain to today's
//! schema. The contract is that *any* input yields a typed `RestoreError`, never
//! a panic or a hang — so the fuzzer just feeds arbitrary bytes and lets
//! libfuzzer flag any crash.
//!
//! The historical payloads are deliberately *rich*: strings, vectors, options,
//! nested structs, and a recursive field (`DocV1::children`). That exercises the
//! serde container-decode and recursion-limit paths — the realistic shapes of
//! persisted aggregate state, and the deep-nesting input that a panic or stack
//! overflow would hide in — not just the unit enum the boundary was first proven
//! on. The chain is three steps (V1 → V2 → V3 → current) so the generated
//! version dispatch and migrate walk are fuzzed across more than one hop.
use ironstate::prelude::*;
use libfuzzer_sys::fuzz_target;
use serde::{Deserialize, Serialize};

/// v1: nested and self-recursive, so arbitrary bytes drive serde_json's
/// container and recursion-limit decode paths (deep input must yield an error,
/// never a stack overflow).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct DocV1 {
    title: String,
    tags: Vec<String>,
    note: Option<String>,
    children: Vec<DocV1>,
}

/// v2: a different nested shape — a vector of sub-structs and a flag.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Revision {
    author: String,
    body: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct DocV2 {
    title: String,
    revisions: Vec<Revision>,
    archived: bool,
}

/// v3: collapses toward the current phase.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct DocV3 {
    status: String,
}

/// The current schema: the state machine itself.
#[derive(StateMachine, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Retired], version = 4, history = [DocV1, DocV2, DocV3])]
enum Doc {
    Draft,
    Live,
    Retired,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum DocEvent {
    Publish,
    Retire,
}

impl TransitionRules for Doc {
    type Event = DocEvent;
    fn transition(&self, event: &DocEvent) -> Option<Doc> {
        use Doc::*;
        use DocEvent::*;
        match (self, event) {
            (Draft, Publish) => Some(Live),
            (Live, Retire) => Some(Retired),
            _ => None,
        }
    }
}

// Total migrations: no indexing, no unwrap, no recursion over `children`, so a
// fuzz failure can only be an ironstate bug — never this harness panicking on
// adversarial decoded data.
impl MigrateFrom<DocV1> for DocV2 {
    fn migrate(old: DocV1) -> DocV2 {
        DocV2 {
            title: old.title,
            revisions: old
                .tags
                .into_iter()
                .map(|tag| Revision {
                    author: String::new(),
                    body: tag,
                })
                .collect(),
            archived: old.note.is_some(),
        }
    }
}

impl MigrateFrom<DocV2> for DocV3 {
    fn migrate(old: DocV2) -> DocV3 {
        DocV3 {
            status: if old.archived {
                "retired".to_string()
            } else {
                "live".to_string()
            },
        }
    }
}

impl MigrateFrom<DocV3> for Doc {
    fn migrate(old: DocV3) -> Doc {
        match old.status.as_str() {
            "retired" => Doc::Retired,
            "live" => Doc::Live,
            _ => Doc::Draft,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = Machine::<Doc>::restore_versioned(data);
});
