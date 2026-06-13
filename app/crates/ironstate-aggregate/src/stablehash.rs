//! Stable hashing: one canonical encoding, two digests.
//!
//! The family owns the *encoding*, not a hash function. A value is encoded to a
//! canonical, platform-independent byte sequence, and that sequence is fed to an
//! adopted engine: a 128-bit fingerprint for detecting accidental divergence
//! ([`Digest128`]), and a collision-resistant digest for adversarial audit
//! settings ([`AuditDigest`], behind the `audit` feature).
//!
//! The encoding is frozen at first release: integers little-endian in fixed
//! width, `usize`/`isize` widened to 8 bytes so 32- and 64-bit targets agree,
//! length prefixes and declaration-order discriminants so distinct values can
//! never collide structurally. Type names are not encoded — a rename is a
//! version bump, not a digest change.

use rustc_stable_hash::FromStableHash;
use rustc_stable_hash::hashers::{SipHasher128Hash, StableSipHasher128};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hasher;

/// A 128-bit fingerprint for detecting *accidental* divergence (replay drift,
/// iteration-order bugs). Not collision-resistant; never publish it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest128(pub [u8; 16]);

impl Digest128 {
    /// The digest as lowercase hex.
    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }
}

impl core::fmt::Display for Digest128 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A collision-resistant digest (BLAKE3) for adversarial settings — the only
/// digest ever published (commit–reveal, audit verification).
#[cfg(feature = "audit")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AuditDigest(pub [u8; 32]);

#[cfg(feature = "audit")]
impl AuditDigest {
    /// The digest as lowercase hex.
    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }
}

#[cfg(feature = "audit")]
impl core::fmt::Display for AuditDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Accumulates a value's canonical byte encoding.
///
/// Hand-written `StableHash` impls drive this directly; the derive generates
/// calls into it. The methods here are the only sanctioned way to write bytes,
/// so every impl stays canonical.
#[derive(Default)]
pub struct CanonicalEncoder {
    buf: Vec<u8>,
}

impl CanonicalEncoder {
    /// A fresh, empty encoder.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// The canonical bytes accumulated so far.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Consume the encoder, returning its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Append raw bytes with no framing (callers must have framed them already).
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Write a length prefix (widened to 8 bytes, little-endian).
    pub fn write_len(&mut self, len: usize) {
        self.buf.extend_from_slice(&(len as u64).to_le_bytes());
    }

    /// Write an enum discriminant (declaration order, 4 bytes, little-endian).
    pub fn write_discriminant(&mut self, discriminant: u32) {
        self.buf.extend_from_slice(&discriminant.to_le_bytes());
    }

    /// Encode one field's value canonically.
    pub fn field<T: StableHash + ?Sized>(&mut self, value: &T) {
        value.encode(self);
    }
}

/// A value with a frozen, platform-independent canonical encoding.
///
/// `x == y` must imply equal encodings, and unequal values must encode
/// differently — the same rule rustc's own stable hashing follows. The derive
/// is convenience; a manual impl is allowed as long as it only uses
/// [`CanonicalEncoder`]'s methods.
pub trait StableHash {
    /// Write this value's canonical bytes into the encoder.
    fn encode(&self, enc: &mut CanonicalEncoder);
}

/// The 128-bit fingerprint of a value's canonical encoding.
pub fn digest128<T: StableHash + ?Sized>(value: &T) -> Digest128 {
    let mut enc = CanonicalEncoder::new();
    value.encode(&mut enc);
    let mut hasher = StableSipHasher128::new();
    hasher.write(enc.bytes());
    let Words(words) = hasher.finish();
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&words[0].to_le_bytes());
    out[8..].copy_from_slice(&words[1].to_le_bytes());
    Digest128(out)
}

/// The collision-resistant digest of a value's canonical encoding.
#[cfg(feature = "audit")]
pub fn audit_digest<T: StableHash + ?Sized>(value: &T) -> AuditDigest {
    let mut enc = CanonicalEncoder::new();
    value.encode(&mut enc);
    AuditDigest(*blake3::hash(enc.bytes()).as_bytes())
}

// Adapter so the SipHasher128 result can be pulled out as raw words.
struct Words([u64; 2]);
impl FromStableHash for Words {
    type Hash = SipHasher128Hash;
    fn from(SipHasher128Hash(hash): SipHasher128Hash) -> Self {
        Words(hash)
    }
}

// --- primitive and std-library impls -------------------------------------

macro_rules! int_impls {
    ($($t:ty),* $(,)?) => {
        $(
            impl StableHash for $t {
                fn encode(&self, enc: &mut CanonicalEncoder) {
                    enc.write_raw(&self.to_le_bytes());
                }
            }
        )*
    };
}
int_impls!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

// usize/isize widen to 8 bytes so digests agree on 32-bit (wasm) and 64-bit targets.
impl StableHash for usize {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_raw(&(*self as u64).to_le_bytes());
    }
}
impl StableHash for isize {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_raw(&(*self as i64).to_le_bytes());
    }
}

impl StableHash for bool {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_raw(&[*self as u8]);
    }
}

impl StableHash for char {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_raw(&(*self as u32).to_le_bytes());
    }
}

impl StableHash for str {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_len(self.len());
        enc.write_raw(self.as_bytes());
    }
}

impl StableHash for String {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        self.as_str().encode(enc);
    }
}

impl StableHash for () {
    fn encode(&self, _enc: &mut CanonicalEncoder) {}
}

impl<T: StableHash + ?Sized> StableHash for &T {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        (**self).encode(enc);
    }
}

impl<T: StableHash + ?Sized> StableHash for Box<T> {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        (**self).encode(enc);
    }
}

impl<T: StableHash> StableHash for [T] {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_len(self.len());
        for item in self {
            item.encode(enc);
        }
    }
}

impl<T: StableHash> StableHash for Vec<T> {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        self.as_slice().encode(enc);
    }
}

impl<T: StableHash, const N: usize> StableHash for [T; N] {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_len(N);
        for item in self {
            item.encode(enc);
        }
    }
}

impl<T: StableHash> StableHash for Option<T> {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        match self {
            None => enc.write_discriminant(0),
            Some(value) => {
                enc.write_discriminant(1);
                value.encode(enc);
            }
        }
    }
}

impl<T: StableHash, E: StableHash> StableHash for Result<T, E> {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        match self {
            Ok(value) => {
                enc.write_discriminant(0);
                value.encode(enc);
            }
            Err(error) => {
                enc.write_discriminant(1);
                error.encode(enc);
            }
        }
    }
}

impl<K: StableHash, V: StableHash> StableHash for BTreeMap<K, V> {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_len(self.len());
        for (key, value) in self {
            key.encode(enc);
            value.encode(enc);
        }
    }
}

impl<T: StableHash> StableHash for BTreeSet<T> {
    fn encode(&self, enc: &mut CanonicalEncoder) {
        enc.write_len(self.len());
        for item in self {
            item.encode(enc);
        }
    }
}

macro_rules! tuple_impls {
    ($(($($name:ident),+);)+) => {
        $(
            impl<$($name: StableHash),+> StableHash for ($($name,)+) {
                #[allow(non_snake_case)]
                fn encode(&self, enc: &mut CanonicalEncoder) {
                    let ($($name,)+) = self;
                    $( $name.encode(enc); )+
                }
            }
        )+
    };
}
tuple_impls! {
    (A);
    (A, B);
    (A, B, C);
    (A, B, C, D);
    (A, B, C, D, E);
    (A, B, C, D, E, F);
}
