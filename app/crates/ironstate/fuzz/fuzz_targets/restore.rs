#![no_main]
//! Fuzz the one place untrusted bytes re-enter ironstate: versioned restore.
//!
//! `restore_versioned` parses a `{version, payload}` envelope, dispatches on the
//! version, decodes the payload, and walks the `MigrateFrom` chain to today's
//! schema. The contract is that *any* input yields a typed `RestoreError`, never
//! a panic — so the fuzzer just feeds arbitrary bytes and lets libfuzzer flag
//! any crash.
use ironstate::prelude::*;
use libfuzzer_sys::fuzz_target;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum DocV1 {
    Draft,
    Live,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum DocV2 {
    Draft,
    Live,
    Retired,
}

#[derive(StateMachine, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Retired], version = 3, history = [DocV1, DocV2])]
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

impl MigrateFrom<DocV1> for DocV2 {
    fn migrate(old: DocV1) -> DocV2 {
        match old {
            DocV1::Draft => DocV2::Draft,
            DocV1::Live => DocV2::Live,
        }
    }
}

impl MigrateFrom<DocV2> for Doc {
    fn migrate(old: DocV2) -> Doc {
        match old {
            DocV2::Draft => Doc::Draft,
            DocV2::Live => Doc::Live,
            DocV2::Retired => Doc::Retired,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = Machine::<Doc>::restore_versioned(data);
});
