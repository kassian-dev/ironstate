//! The `Versioned` derive on an event enum: a stored event written by an older
//! version decodes and migrates forward through the `MigrateFrom` chain.
#![cfg(feature = "restore")]

use ironstate_aggregate::{MigrateFrom, RestoreError, Versioned};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum MatchEventV1 {
    Joined,
    Left,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum MatchEventV2 {
    Joined,
    Left,
    Renamed,
}

#[derive(ironstate_aggregate::Versioned, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[versioned(version = 3, history = [MatchEventV1, MatchEventV2])]
enum MatchEvent {
    Joined,
    Left,
    Renamed,
    Kicked,
}

impl MigrateFrom<MatchEventV1> for MatchEventV2 {
    fn migrate(old: MatchEventV1) -> MatchEventV2 {
        match old {
            MatchEventV1::Joined => MatchEventV2::Joined,
            MatchEventV1::Left => MatchEventV2::Left,
        }
    }
}
impl MigrateFrom<MatchEventV2> for MatchEvent {
    fn migrate(old: MatchEventV2) -> MatchEvent {
        match old {
            MatchEventV2::Joined => MatchEvent::Joined,
            MatchEventV2::Left => MatchEvent::Left,
            MatchEventV2::Renamed => MatchEvent::Renamed,
        }
    }
}

fn envelope(version: u32, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "version": version, "payload": payload })).unwrap()
}

#[test]
fn v1_event_migrates_through_the_chain() {
    // MatchEventV1::Left -> MatchEventV2::Left -> MatchEvent::Left
    let bytes = envelope(1, serde_json::json!("Left"));
    assert_eq!(
        MatchEvent::restore_versioned(&bytes).unwrap(),
        MatchEvent::Left
    );
}

#[test]
fn current_version_loads_directly() {
    let bytes = envelope(3, serde_json::json!("Kicked"));
    assert_eq!(
        MatchEvent::restore_versioned(&bytes).unwrap(),
        MatchEvent::Kicked
    );
}

#[test]
fn newer_than_binary_is_typed() {
    let bytes = envelope(4, serde_json::json!("Joined"));
    assert!(matches!(
        MatchEvent::restore_versioned(&bytes).unwrap_err(),
        RestoreError::NewerThanBinary {
            found: 4,
            supports: 3
        },
    ));
}

#[test]
fn version_constant_is_exposed() {
    assert_eq!(<MatchEvent as Versioned>::VERSION, 3);
}
