//! Deterministic context and entropy.
//!
//! `decide` is the only function permitted to draw entropy, and it draws from a
//! journal-owned source addressed by a counter — so a draw is a pure function
//! of `(seed, position)`, replay seeks in O(1), and a failed command can rewind
//! exactly. There is no float draw and no clock: the API simply cannot express
//! the non-deterministic inputs that would break replay.

use core::ops::Range;
use rand_chacha::ChaCha12Rng;
use rand_chacha::rand_core::{Rng, SeedableRng};

/// A position in the entropy stream — the count of words drawn so far.
///
/// It is recorded per append because replay consumes no entropy (only `decide`
/// draws), so the position cannot be recomputed from events and must be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawPos(pub u64);

/// Logical time as data, advanced by tick events. Never read from a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LogicalTime(pub u64);

/// The journal-owned secret a stream is derived from.
///
/// Never lives in state, and is never serialized by the family — its storage is
/// the application's concern. Its `Debug` is redacted so it cannot leak into a
/// log or a panic message, and it has no `Display`.
pub struct Seed(pub [u8; 32]);

impl core::fmt::Debug for Seed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Seed(<redacted>)")
    }
}

/// A counter-addressable source of deterministic randomness.
///
/// The derived draws (`draw_range`, `draw_below`, `shuffle_len`) are provided
/// here so every source shares one unbiased algorithm — that shared algorithm
/// is part of the determinism contract, not an implementation detail.
pub trait EntropySource {
    /// Draw the next 64-bit word.
    fn draw_u64(&mut self) -> u64;

    /// Seek to an absolute position. O(1) for the seeded source.
    fn seek(&mut self, pos: DrawPos);

    /// The current position.
    fn draws(&self) -> DrawPos;

    /// An independent fork at the current position whose draws are uncounted
    /// and unjournaled — for speculative checks like `why_not`.
    fn probe(&self) -> Box<dyn EntropySource>;

    /// A uniformly-random value in `range`, with no modulo bias.
    ///
    /// Rejection sampling over the smallest covering power of two: it draws a
    /// word, masks it, and retries on a value outside the range.
    fn draw_range(&mut self, range: Range<u64>) -> u64 {
        let span = range
            .end
            .checked_sub(range.start)
            .expect("draw_range needs end >= start");
        assert!(span > 0, "draw_range needs a non-empty range");
        let bits = u64::BITS - (span - 1).leading_zeros();
        let mask = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        loop {
            let candidate = self.draw_u64() & mask;
            if candidate < span {
                return range.start + candidate;
            }
        }
    }

    /// True with probability exactly `num/den` (one `draw_range`).
    fn draw_below(&mut self, num: u64, den: u64) -> bool {
        assert!(den > 0, "draw_below needs a positive denominator");
        assert!(num <= den, "draw_below needs num <= den");
        self.draw_range(0..den) < num
    }

    /// A Fisher–Yates permutation of `0..len`, issuing exactly `len - 1` draws.
    fn shuffle_len(&mut self, len: usize) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..len).collect();
        for i in (1..len).rev() {
            let j = self.draw_range(0..(i as u64 + 1)) as usize;
            indices.swap(i, j);
        }
        indices
    }
}

/// Extension method that shuffles a slice in place using the same Fisher–Yates
/// draw sequence as [`EntropySource::shuffle_len`].
///
/// Kept off the object-safe trait because it is generic; it works on
/// `dyn EntropySource` all the same.
pub trait EntropySourceExt: EntropySource {
    /// Shuffle `slice` in place, consuming `slice.len() - 1` draws.
    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.draw_range(0..(i as u64 + 1)) as usize;
            slice.swap(i, j);
        }
    }
}

impl<E: EntropySource + ?Sized> EntropySourceExt for E {}

/// The seeded source: a counter-addressable ChaCha12 stream.
///
/// Deliberately not `Clone` — duplicating a key stream is a footgun the rand
/// crate now forbids. `probe` and rewind reconstruct a stream from the seed and
/// a position instead, which is exactly the O(1) seek the contract requires.
pub struct SeededEntropy {
    rng: ChaCha12Rng,
    seed: [u8; 32],
}

impl SeededEntropy {
    /// A stream positioned at `pos`. The seek is O(1).
    pub fn at(seed: &Seed, pos: DrawPos) -> Self {
        let mut rng = ChaCha12Rng::from_seed(seed.0);
        rng.set_word_pos(pos.0 as u128);
        Self { rng, seed: seed.0 }
    }

    /// A stream at the start of the keystream.
    pub fn from_seed(seed: &Seed) -> Self {
        Self::at(seed, DrawPos(0))
    }
}

impl EntropySource for SeededEntropy {
    fn draw_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    fn seek(&mut self, pos: DrawPos) {
        self.rng.set_word_pos(pos.0 as u128);
    }

    fn draws(&self) -> DrawPos {
        DrawPos(self.rng.get_word_pos() as u64)
    }

    fn probe(&self) -> Box<dyn EntropySource> {
        Box::new(SeededEntropy::at(&Seed(self.seed), self.draws()))
    }
}

/// A ready-made deterministic context: borrowed entropy, an actor identity, and
/// logical time. Define your own `Ctx` if this shape does not fit — the family
/// never requires it.
pub struct DeterministicCtx<'a, Actor> {
    /// The journal-owned entropy stream.
    pub entropy: &'a mut dyn EntropySource,
    /// Who issued the command.
    pub actor: Actor,
    /// Logical time, as data.
    pub now: LogicalTime,
}

impl<'a, Actor: Clone> DeterministicCtx<'a, Actor> {
    /// A speculative copy whose entropy is a `probe` — for `why_not`, so a
    /// legality check never advances the journaled stream.
    pub fn probing(&self) -> OwnedDeterministicCtx<Actor> {
        OwnedDeterministicCtx {
            entropy: self.entropy.probe(),
            actor: self.actor.clone(),
            now: self.now,
        }
    }
}

/// A `DeterministicCtx` that owns its (probed) entropy.
pub struct OwnedDeterministicCtx<Actor> {
    /// An owned, uncounted entropy fork.
    pub entropy: Box<dyn EntropySource>,
    /// Who issued the command.
    pub actor: Actor,
    /// Logical time, as data.
    pub now: LogicalTime,
}

/// Lets the persistent loop reach the entropy inside an opaque `Ctx` to record
/// and rewind positions. Entropy-free contexts return `None`.
pub trait CtxEntropy {
    /// The context's entropy stream, if it has one.
    fn entropy_mut(&mut self) -> Option<&mut dyn EntropySource>;
}

impl<Actor> CtxEntropy for DeterministicCtx<'_, Actor> {
    fn entropy_mut(&mut self) -> Option<&mut dyn EntropySource> {
        Some(&mut *self.entropy)
    }
}

impl<Actor> CtxEntropy for OwnedDeterministicCtx<Actor> {
    fn entropy_mut(&mut self) -> Option<&mut dyn EntropySource> {
        Some(&mut *self.entropy)
    }
}
