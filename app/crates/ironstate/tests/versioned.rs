//! Versioned restore: a stored value written by an older schema migrates
//! forward through the `MigrateFrom` chain when loaded.
#![cfg(feature = "restore")]

use ironstate::prelude::*;
use serde::{Deserialize, Serialize};

// The two retired schemas. Each is an ordinary, independently-testable type.
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

// The current schema (version 3), with the prior versions listed oldest-first.
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

fn envelope(version: u32, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "version": version, "payload": payload })).unwrap()
}

#[test]
fn current_version_loads_directly() {
    let bytes = envelope(3, serde_json::json!("Live"));
    let m = Machine::<Doc>::restore_versioned(&bytes).unwrap();
    assert_eq!(m.state(), &Doc::Live);
}

#[test]
fn v1_migrates_through_the_whole_chain() {
    // DocV1::Live -> DocV2::Live -> Doc::Live
    let bytes = envelope(1, serde_json::json!("Live"));
    let m = Machine::<Doc>::restore_versioned(&bytes).unwrap();
    assert_eq!(m.state(), &Doc::Live);
}

#[test]
fn v2_migrates_one_step() {
    let bytes = envelope(2, serde_json::json!("Retired"));
    let m = Machine::<Doc>::restore_versioned(&bytes).unwrap();
    assert_eq!(m.state(), &Doc::Retired);
}

#[test]
fn newer_than_binary_is_typed() {
    let bytes = envelope(4, serde_json::json!("Live"));
    let err = Machine::<Doc>::restore_versioned(&bytes).unwrap_err();
    match err {
        RestoreError::NewerThanBinary { found, supports } => {
            assert_eq!(found, 4);
            assert_eq!(supports, 3);
        }
        other => panic!("expected NewerThanBinary, got {other:?}"),
    }
}

#[test]
fn unknown_version_is_typed() {
    let bytes = envelope(0, serde_json::json!("Live"));
    let err = Machine::<Doc>::restore_versioned(&bytes).unwrap_err();
    assert!(matches!(err, RestoreError::UnknownVersion { found: 0 }));
}

#[test]
fn version_constant_is_exposed() {
    assert_eq!(<Doc as Versioned>::VERSION, 3);
}
