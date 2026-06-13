//! Behavioral tests for the canonical encoding and its digests: injectivity at
//! the ambiguity points, derive codegen, skip, and PartialEq conformance.
#![cfg(feature = "stablehash")]

use ironstate_aggregate::{CanonicalEncoder, StableHash, digest128};
use std::collections::BTreeMap;

fn bytes_of<T: StableHash + ?Sized>(value: &T) -> Vec<u8> {
    let mut enc = CanonicalEncoder::new();
    value.encode(&mut enc);
    enc.into_bytes()
}

#[test]
fn length_prefixes_disambiguate_concatenation() {
    // The classic ambiguity: without length prefixes these two tuples would
    // encode to the same bytes.
    let a = ("ab".to_string(), "c".to_string());
    let b = ("a".to_string(), "bc".to_string());
    assert_ne!(bytes_of(&a), bytes_of(&b));
    assert_ne!(digest128(&a), digest128(&b));
}

#[test]
fn discriminants_distinguish_variants() {
    let none = None::<u8>;
    let some_zero = Some(0u8);
    // None is discriminant 0 with no payload; Some(0) is discriminant 1 + a zero
    // byte — distinct encodings despite the "empty-ish" payload.
    assert_ne!(bytes_of(&none), bytes_of(&some_zero));
}

#[test]
fn empty_and_absent_collections_differ() {
    let empty: Vec<u8> = Vec::new();
    let one = vec![0u8];
    assert_ne!(bytes_of(&empty), bytes_of(&one));
    // An empty vec still writes its length prefix, so it is not the empty string.
    assert_eq!(bytes_of(&empty), 0u64.to_le_bytes().to_vec());
}

#[test]
fn usize_is_widened_to_eight_bytes() {
    // Same numeric value as usize and u64 must encode identically — that is what
    // makes 32- and 64-bit targets agree.
    assert_eq!(bytes_of(&7usize), bytes_of(&7u64));
    assert_eq!(bytes_of(&5usize).len(), 8);
}

#[test]
fn btreemap_encodes_in_key_order() {
    let mut forward = BTreeMap::new();
    forward.insert(1u32, "a".to_string());
    forward.insert(2u32, "b".to_string());

    let mut reversed = BTreeMap::new();
    reversed.insert(2u32, "b".to_string());
    reversed.insert(1u32, "a".to_string());

    // Insertion order differs but key order is canonical, so digests match.
    assert_eq!(digest128(&forward), digest128(&reversed));
}

// --- derive ---------------------------------------------------------------

#[derive(ironstate_aggregate::StableHash, PartialEq, Clone, Debug)]
struct Point {
    x: i32,
    y: i32,
    label: String,
}

#[derive(ironstate_aggregate::StableHash, PartialEq, Clone, Debug)]
enum Shape {
    Dot,
    Circle(u32),
    Rect { w: u32, h: u32 },
}

#[derive(ironstate_aggregate::StableHash, PartialEq, Clone, Debug)]
struct WithCache {
    id: u64,
    // Not part of identity, and would otherwise be a non-deterministic type.
    #[stable_hash(skip)]
    cached_render: String,
}

#[test]
fn derive_distinguishes_field_changes() {
    let a = Point {
        x: 1,
        y: 2,
        label: "p".into(),
    };
    let b = Point {
        x: 1,
        y: 3,
        label: "p".into(),
    };
    assert_ne!(digest128(&a), digest128(&b));
}

#[test]
fn derive_distinguishes_enum_variants() {
    assert_ne!(digest128(&Shape::Dot), digest128(&Shape::Circle(0)));
    assert_ne!(
        digest128(&Shape::Circle(1)),
        digest128(&Shape::Rect { w: 1, h: 1 })
    );
}

#[test]
fn skip_excludes_field_from_identity() {
    let a = WithCache {
        id: 7,
        cached_render: "one".into(),
    };
    let b = WithCache {
        id: 7,
        cached_render: "two".into(),
    };
    // The skipped field differs but identity (id) is the same.
    assert_eq!(digest128(&a), digest128(&b));
}

#[test]
fn partial_eq_conformance() {
    // x == y must imply equal digests; unequal values must differ.
    let x = Point {
        x: 4,
        y: 5,
        label: "q".into(),
    };
    let y = x.clone();
    assert_eq!(x, y);
    assert_eq!(digest128(&x), digest128(&y));

    let z = Point {
        x: 4,
        y: 5,
        label: "r".into(),
    };
    assert_ne!(x, z);
    assert_ne!(digest128(&x), digest128(&z));
}

// Run with `--ignored --nocapture` to print fresh golden values; they are then
// frozen in tests/golden.rs and never regenerated to make a test pass.
#[test]
#[ignore = "prints golden vectors for the frozen golden test"]
fn emit_golden_vectors() {
    println!("u64_42        = {}", digest128(&42u64).to_hex());
    println!("str_hello     = {}", digest128(&"hello").to_hex());
    println!("vec_1_2_3     = {}", digest128(&vec![1u32, 2, 3]).to_hex());
    println!("some_7        = {}", digest128(&Some(7u8)).to_hex());
    println!("none_u8       = {}", digest128(&None::<u8>).to_hex());
    println!(
        "point         = {}",
        digest128(&Point {
            x: -1,
            y: 2,
            label: "p".into()
        })
        .to_hex()
    );
    println!("shape_circle  = {}", digest128(&Shape::Circle(9)).to_hex());
    println!(
        "shape_rect    = {}",
        digest128(&Shape::Rect { w: 3, h: 4 }).to_hex()
    );

    #[cfg(feature = "audit")]
    {
        use ironstate_aggregate::audit_digest;
        println!("audit_u64_42  = {}", audit_digest(&42u64).to_hex());
        println!("audit_str     = {}", audit_digest(&"hello").to_hex());
        println!(
            "audit_point   = {}",
            audit_digest(&Point {
                x: -1,
                y: 2,
                label: "p".into()
            })
            .to_hex()
        );
    }
}
