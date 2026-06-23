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

/// Assert that a custom [`EntropySource`] obeys the determinism contract.
///
/// **Why this exists.** The family's promise — same seed, same outcome, on every
/// machine — only holds if every entropy source draws values the same way. Most
/// sources inherit that for free from the default `draw_range` / `draw_below` /
/// `shuffle_len`. But the trait lets you *override* those — to keep a stream your
/// golden vectors are already pinned to, or to wrap a different generator — and
/// nothing in an ordinary test suite re-checks that an override stays
/// well-behaved: an in-range, *covering* draw; a real permutation; a seek that
/// lands you exactly back — including the backward rewind a failed command uses.
/// This runs those checks in one call, the same way
/// [`journal_contract_test!`](crate) proves a custom journal adapter conforms.
///
/// It checks *properties*, not specific values, so it passes for any correct
/// algorithm — the default bit-mask scheme or your own modulo/rejection one. It
/// verifies a draw stays in range and *covers* that range (a constant or
/// truncated draw is caught), not that it is statistically uniform — a subtle
/// modulo bias is the golden vector's job, not this. Likewise it proves `seek`
/// *reconstructs* the stream, not that it is O(1) (that is a property of
/// `SeededEntropy`, not something a conformance check can time). It deliberately
/// does **not** pin your stream's exact bytes; keep a separate golden-vector test
/// (a frozen `draw_u64` sequence) for that, since those values are yours, not the
/// contract's.
///
/// `fresh` must return a brand-new source at the start of the *same* seed on
/// every call — the checks reconstruct and seek it.
///
/// # What it proves
/// - **same seed, same stream** — two fresh sources agree word-for-word;
/// - **seek reconstructs the stream** — draw to a position, seek a fresh source
///   there, and the next value matches; and a backward seek from an advanced
///   position rewinds (the replay/rewind contract);
/// - **`probe` is pure** — it leaves the parent's position untouched and forks
///   from exactly that position;
/// - **`draw_range(a..b)` stays in `[a, b)` and covers it** — a unit range
///   `x..x+1` is the constant `x`, and a small range reaches every value (a
///   constant or truncated draw is caught);
/// - **`draw_below(0, d)` is never true; `draw_below(d, d)` always is**;
/// - **`shuffle_len(n)` is a permutation of `0..n`**, agrees with `shuffle` over
///   the identity, and `shuffle_len(1)` draws nothing.
///
/// # Examples
/// ```
/// use ironstate_aggregate::{DrawPos, EntropySource, assert_entropy_contract};
///
/// // A tiny counter-addressable source (splitmix64 at a position) that overrides
/// // `draw_range` with modulo-zone rejection instead of the default bit-mask —
/// // exactly the kind of override worth contract-checking.
/// struct Splitmix {
///     seed: u64,
///     pos: u64,
/// }
///
/// impl EntropySource for Splitmix {
///     fn draw_u64(&mut self) -> u64 {
///         self.pos += 1;
///         let mut z = self.seed.wrapping_add(self.pos.wrapping_mul(0x9E37_79B9_7F4A_7C15));
///         z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
///         z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
///         z ^ (z >> 31)
///     }
///     fn seek(&mut self, pos: DrawPos) {
///         self.pos = pos.0;
///     }
///     fn draws(&self) -> DrawPos {
///         DrawPos(self.pos)
///     }
///     fn probe(&self) -> Box<dyn EntropySource> {
///         Box::new(Splitmix { seed: self.seed, pos: self.pos })
///     }
///     // Modulo-zone rejection: unbiased, but a different stream from the default.
///     fn draw_range(&mut self, range: core::ops::Range<u64>) -> u64 {
///         let n = range.end - range.start;
///         let zone = u64::MAX - (u64::MAX % n);
///         loop {
///             let v = self.draw_u64();
///             if v < zone {
///                 return range.start + v % n;
///             }
///         }
///     }
/// }
///
/// // One call verifies the override is still in-range, covering, seekable, and
/// // forkable. It panics with a teaching message if any property breaks.
/// assert_entropy_contract(|| Splitmix { seed: 7, pos: 0 });
/// ```
///
/// # Panics
/// On the first property that fails, with a message naming the property and the
/// offending values — so a broken source points straight at the fix.
pub fn assert_entropy_contract<E: EntropySource>(fresh: impl Fn() -> E) {
    // Same seed, same stream.
    let (mut a, mut b) = (fresh(), fresh());
    for word in 0..32 {
        let (x, y) = (a.draw_u64(), b.draw_u64());
        assert_eq!(
            x, y,
            "same seed must give the same stream, but word {word} differed: \
             {x:#018x} != {y:#018x}",
        );
    }

    // seek round-trips: a fresh source seeked to a position resumes the stream.
    let mut original = fresh();
    for _ in 0..5 {
        original.draw_u64();
    }
    let pos = original.draws();
    let next = original.draw_u64();
    let mut seeked = fresh();
    seeked.seek(pos);
    assert_eq!(
        seeked.draw_u64(),
        next,
        "seek must reconstruct the stream: a fresh source seeked to {pos:?} drew a \
         different next value than the original",
    );

    // seek rewinds, not only fast-forwards: the abort path seeks backward to undo
    // a rejected command. Remember a position and the value at it, draw well past
    // it, then seek back — the stream must resume from there.
    let mut e = fresh();
    e.draw_u64();
    e.draw_u64();
    let early = e.draws();
    let at_early = e.draw_u64();
    for _ in 0..5 {
        e.draw_u64();
    }
    e.seek(early);
    assert_eq!(
        e.draw_u64(),
        at_early,
        "seek must rewind to an earlier position {early:?}, not only fast-forward",
    );

    // probe is pure: it does not advance the parent, and forks from exactly here.
    let mut parent = fresh();
    parent.draw_u64();
    let forked_at = parent.draws();
    let mut probe = parent.probe();
    let probed = [probe.draw_u64(), probe.draw_u64()];
    assert_eq!(
        parent.draws(),
        forked_at,
        "probe must not advance the parent (it is an uncounted fork)",
    );
    let mut rebuilt = fresh();
    rebuilt.seek(forked_at);
    assert_eq!(
        [rebuilt.draw_u64(), rebuilt.draw_u64()],
        probed,
        "probe must fork from the parent's current position {forked_at:?}",
    );

    // draw_range stays in bounds; a unit range is constant.
    let mut e = fresh();
    for _ in 0..1000 {
        let v = e.draw_range(10..20);
        assert!(
            (10..20).contains(&v),
            "draw_range(10..20) must stay in [10, 20), drew {v}",
        );
    }
    let mut e = fresh();
    for _ in 0..8 {
        let v = e.draw_range(5..6);
        assert_eq!(v, 5, "draw_range(5..6) must be the constant 5, drew {v}");
    }

    // draw_range covers its range: a draw that stays in bounds but collapses to
    // one value (a constant or truncated source) is still broken, and the
    // in-range check alone would pass it. Not a uniformity test — just that every
    // value is reachable.
    let mut e = fresh();
    let mut seen = [false; 6];
    for _ in 0..200 {
        let v = e.draw_range(0..6);
        assert!(v < 6, "draw_range(0..6) must stay in [0, 6), drew {v}");
        seen[v as usize] = true;
    }
    assert!(
        seen.iter().all(|&hit| hit),
        "draw_range(0..6) must reach every value over many draws, but missed some: {seen:?}",
    );

    // draw_below boundaries: 0/den is never true, den/den always is.
    let mut e = fresh();
    for _ in 0..100 {
        assert!(!e.draw_below(0, 10), "draw_below(0, 10) must never be true");
    }
    let mut e = fresh();
    for _ in 0..100 {
        assert!(
            e.draw_below(10, 10),
            "draw_below(10, 10) must always be true"
        );
    }

    // shuffle_len is a permutation, agrees with shuffle, and shuffle_len(1) draws nothing.
    let mut e = fresh();
    let perm = e.shuffle_len(8);
    let mut sorted = perm.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        (0..8).collect::<Vec<_>>(),
        "shuffle_len(8) must be a permutation of 0..8, got {perm:?}",
    );
    let mut e = fresh();
    let via_len = e.shuffle_len(10);
    let mut identity: Vec<usize> = (0..10).collect();
    let mut e = fresh();
    e.shuffle(&mut identity);
    assert_eq!(
        via_len, identity,
        "shuffle_len and shuffle must issue the same permutation",
    );
    let mut e = fresh();
    let start = e.draws();
    let _ = e.shuffle_len(1);
    assert_eq!(
        e.draws(),
        start,
        "shuffle_len(1) must draw nothing, but the position advanced",
    );
}
