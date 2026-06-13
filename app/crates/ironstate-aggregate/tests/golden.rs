//! Frozen golden digest vectors.
//!
//! These pin the canonical encoding forever. They were generated once with the
//! code under test and must never be regenerated to make a failing test green —
//! a change here means the encoding changed, which is a frozen-encoding-rule
//! violation and a breaking, audit-visible event. The same vectors are checked
//! on x86_64 and wasm32, so an upstream engine change surfaces here rather than
//! shipping as silent cross-platform divergence.
#![cfg(feature = "stablehash")]

use ironstate_aggregate::digest128;

#[derive(ironstate_aggregate::StableHash)]
struct Point {
    x: i32,
    y: i32,
    label: String,
}

#[derive(ironstate_aggregate::StableHash)]
enum Shape {
    #[allow(dead_code)]
    Dot,
    Circle(u32),
    Rect {
        w: u32,
        h: u32,
    },
}

#[test]
fn digest128_golden_vectors() {
    assert_eq!(
        digest128(&42u64).to_hex(),
        "bd2c0b84d2b9a938af3b0c2a8000e3d3"
    );
    assert_eq!(
        digest128(&"hello").to_hex(),
        "b085c71651f2c30b5dae04af94834582"
    );
    assert_eq!(
        digest128(&vec![1u32, 2, 3]).to_hex(),
        "7b685b92156f4190354dd34e7d4a3d59"
    );
    assert_eq!(
        digest128(&Some(7u8)).to_hex(),
        "3c73d29626d2db35f4683b567205351a"
    );
    assert_eq!(
        digest128(&None::<u8>).to_hex(),
        "e106c3f5e281204ad514b40dd1f9b5dc"
    );
    assert_eq!(
        digest128(&Point {
            x: -1,
            y: 2,
            label: "p".into()
        })
        .to_hex(),
        "431ab104311661de90c2bee3a155fe88"
    );
    assert_eq!(
        digest128(&Shape::Circle(9)).to_hex(),
        "6bc7a743124645b98da2c458b8b1daeb"
    );
    assert_eq!(
        digest128(&Shape::Rect { w: 3, h: 4 }).to_hex(),
        "0c1b417b19cd0f79a5163ea539d4609e"
    );
}

#[cfg(feature = "audit")]
#[test]
fn audit_digest_golden_vectors() {
    use ironstate_aggregate::audit_digest;
    assert_eq!(
        audit_digest(&42u64).to_hex(),
        "fae624a6c2dcaa946ec81bbee9d0ee5c298c00955d3f889057e7ac83ed2dd170"
    );
    assert_eq!(
        audit_digest(&"hello").to_hex(),
        "cc2104c68d62617ffb94126d6c6962b155add154b79d0b9df047c2fd4333e537"
    );
    assert_eq!(
        audit_digest(&Point {
            x: -1,
            y: 2,
            label: "p".into()
        })
        .to_hex(),
        "b9867f9eb9ad0d7f949503e07c2d02153e66c2c13cb09ee499acc0eceb1ad5b7"
    );
}
