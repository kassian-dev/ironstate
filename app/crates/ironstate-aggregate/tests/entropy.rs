//! The `EntropySource` contract, exercised three ways: the reference
//! `SeededEntropy` is proven against it, its frozen golden stream is pinned, and
//! a battery of planted defects proves the contract check actually catches a
//! broken source (test-the-testers).

use core::ops::Range;
use ironstate_aggregate::{DrawPos, EntropySource, Seed, SeededEntropy, assert_entropy_contract};

fn seed() -> Seed {
    Seed([7u8; 32])
}

#[test]
fn seeded_entropy_obeys_the_contract() {
    // The reference source passes its own contract — and this single call is
    // exactly what a crate with a custom source writes to verify theirs.
    assert_entropy_contract(|| SeededEntropy::from_seed(&seed()));
}

#[test]
fn golden_stream_vectors() {
    // Frozen: the first words of the ChaCha12 stream for seed [7; 32]. These pin
    // the engine's output across platforms; a mismatch is upstream drift, not a
    // value to regenerate. Run `emit_golden_stream --ignored --nocapture` to see
    // freshly-computed values when intentionally rebaselining. The contract test
    // above proves the *properties*; these pin the exact *bytes* — both matter,
    // because the contract holds for any correct algorithm, not just this stream.
    let mut e = SeededEntropy::from_seed(&seed());
    let words: Vec<u64> = (0..4).map(|_| e.draw_u64()).collect();
    assert_eq!(
        words,
        vec![
            0x20cb_c085_7889_92f6,
            0xbc15_0b6a_10cd_e4a3,
            0x8929_737e_f194_0736,
            0x4174_ec73_8879_b009,
        ]
    );
    // The position after four u64 draws is 8 words.
    assert_eq!(e.draws(), DrawPos(8));
}

#[test]
#[ignore = "prints golden stream values for rebaselining"]
fn emit_golden_stream() {
    let mut e = SeededEntropy::from_seed(&seed());
    for _ in 0..4 {
        println!("0x{:016x}", e.draw_u64());
    }
    println!("pos = {:?}", e.draws());
}

// --- test-the-testers -----------------------------------------------------
//
// A planted defect must be CAUGHT by the contract check, and a clean source must
// pass. `Planted` is a correct counter-addressable source (splitmix64 at a
// position) with one toggleable flaw; each test injects a single defect and
// asserts the matching contract property fires. Without these, a refactor could
// hollow out `assert_entropy_contract` and nothing would notice.

#[derive(Clone, Copy)]
enum Defect {
    None,
    /// `draw_range` skips the rejection step and escapes its bounds.
    OutOfRange,
    /// `seek` silently ignores the requested position.
    DeadSeek,
    /// `probe` forks one position too far ahead.
    ProbeDrift,
    /// `shuffle_len` returns a non-permutation.
    BadShuffle,
    /// `draw_below` is off by one at the boundary: `<=` makes `draw_below(0, d)`
    /// occasionally true.
    BadBelow,
    /// `shuffle_len`'s Fisher–Yates loop starts one too low, so it draws for
    /// `len == 1` instead of issuing zero draws.
    DrawingShuffle,
    /// `draw_range` collapses to the low bound — in range, but never varies.
    DegenerateDraw,
    /// `seek` only ever moves forward; a backward seek (rewind) is ignored.
    ForwardOnlySeek,
}

struct Planted {
    pos: u64,
    defect: Defect,
}

impl Planted {
    fn new(defect: Defect) -> Self {
        Planted { pos: 0, defect }
    }
}

fn splitmix(pos: u64) -> u64 {
    let mut z = 7u64.wrapping_add(pos.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl EntropySource for Planted {
    fn draw_u64(&mut self) -> u64 {
        self.pos += 1;
        splitmix(self.pos)
    }
    fn seek(&mut self, pos: DrawPos) {
        match self.defect {
            Defect::DeadSeek => {}                            // ignores every seek
            Defect::ForwardOnlySeek if pos.0 < self.pos => {} // refuses to rewind
            _ => self.pos = pos.0,
        }
    }
    fn draws(&self) -> DrawPos {
        DrawPos(self.pos)
    }
    fn probe(&self) -> Box<dyn EntropySource> {
        let pos = match self.defect {
            Defect::ProbeDrift => self.pos + 1,
            _ => self.pos,
        };
        Box::new(Planted {
            pos,
            defect: self.defect,
        })
    }
    fn draw_range(&mut self, range: Range<u64>) -> u64 {
        if matches!(self.defect, Defect::OutOfRange) {
            return range.start + self.draw_u64(); // unmasked: escapes [a, b)
        }
        if matches!(self.defect, Defect::DegenerateDraw) {
            return range.start; // in range, but constant — never covers the range
        }
        let n = range.end - range.start;
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.draw_u64();
            if v < zone {
                return range.start + v % n;
            }
        }
    }
    fn draw_below(&mut self, num: u64, den: u64) -> bool {
        if matches!(self.defect, Defect::BadBelow) {
            return self.draw_range(0..den) <= num; // off-by-one: draw_below(0, d) can be true
        }
        self.draw_range(0..den) < num
    }
    fn shuffle_len(&mut self, len: usize) -> Vec<usize> {
        if matches!(self.defect, Defect::BadShuffle) {
            return vec![0; len]; // not a permutation
        }
        // Fisher–Yates stops before index 0; DrawingShuffle starts one too low, so
        // it issues a (no-op) draw for the last element — a real permutation, but
        // shuffle_len(1) is no longer draw-free.
        let lo = if matches!(self.defect, Defect::DrawingShuffle) {
            0
        } else {
            1
        };
        let mut indices: Vec<usize> = (0..len).collect();
        for i in (lo..len).rev() {
            let j = self.draw_range(0..(i as u64 + 1)) as usize;
            indices.swap(i, j);
        }
        indices
    }
}

#[test]
fn contract_passes_a_correct_source() {
    assert_entropy_contract(|| Planted::new(Defect::None));
}

#[test]
#[should_panic(expected = "draw_range(10..20)")]
fn contract_catches_an_out_of_range_draw() {
    assert_entropy_contract(|| Planted::new(Defect::OutOfRange));
}

#[test]
#[should_panic(expected = "seek must reconstruct")]
fn contract_catches_a_dead_seek() {
    assert_entropy_contract(|| Planted::new(Defect::DeadSeek));
}

#[test]
#[should_panic(expected = "probe must fork")]
fn contract_catches_a_drifting_probe() {
    assert_entropy_contract(|| Planted::new(Defect::ProbeDrift));
}

#[test]
#[should_panic(expected = "permutation")]
fn contract_catches_a_non_permutation_shuffle() {
    assert_entropy_contract(|| Planted::new(Defect::BadShuffle));
}

#[test]
#[should_panic(expected = "draw_below(0, 10) must never be true")]
fn contract_catches_an_off_by_one_draw_below() {
    assert_entropy_contract(|| Planted::new(Defect::BadBelow));
}

#[test]
#[should_panic(expected = "shuffle_len(1) must draw nothing")]
fn contract_catches_a_drawing_shuffle_len() {
    assert_entropy_contract(|| Planted::new(Defect::DrawingShuffle));
}

#[test]
#[should_panic(expected = "must reach every value")]
fn contract_catches_a_degenerate_draw() {
    assert_entropy_contract(|| Planted::new(Defect::DegenerateDraw));
}

#[test]
#[should_panic(expected = "must rewind")]
fn contract_catches_a_forward_only_seek() {
    assert_entropy_contract(|| Planted::new(Defect::ForwardOnlySeek));
}
