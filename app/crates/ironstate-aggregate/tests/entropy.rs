//! Entropy invariants: determinism, O(1) seek/reconstruction, probe purity,
//! unbiased draws, and frozen golden stream vectors.

use ironstate_aggregate::{DrawPos, EntropySource, EntropySourceExt, Seed, SeededEntropy};

fn seed() -> Seed {
    Seed([7u8; 32])
}

#[test]
fn same_seed_same_stream() {
    let mut a = SeededEntropy::from_seed(&seed());
    let mut b = SeededEntropy::from_seed(&seed());
    for _ in 0..16 {
        assert_eq!(a.draw_u64(), b.draw_u64());
    }
}

#[test]
fn seek_reconstructs_the_stream() {
    let mut a = SeededEntropy::from_seed(&seed());
    let _ = a.draw_u64();
    let _ = a.draw_u64();
    let pos = a.draws();
    let next = a.draw_u64();

    // A fresh stream seeked to `pos` produces the same next value — O(1) seek.
    let mut b = SeededEntropy::at(&seed(), pos);
    assert_eq!(b.draw_u64(), next);
}

#[test]
fn probe_does_not_advance_the_parent() {
    let mut parent = SeededEntropy::from_seed(&seed());
    let _ = parent.draw_u64();
    let before = parent.draws();

    let mut probe = parent.probe();
    let probed = [probe.draw_u64(), probe.draw_u64()];

    // The parent's position is untouched...
    assert_eq!(parent.draws(), before);
    // ...and the probe forked from exactly there.
    let mut check = SeededEntropy::at(&seed(), before);
    assert_eq!([check.draw_u64(), check.draw_u64()], probed);
}

#[test]
fn draw_range_stays_in_bounds() {
    let mut e = SeededEntropy::from_seed(&seed());
    for _ in 0..1000 {
        let v = e.draw_range(10..20);
        assert!((10..20).contains(&v));
    }
}

#[test]
fn draw_range_single_value_is_constant() {
    let mut e = SeededEntropy::from_seed(&seed());
    for _ in 0..8 {
        assert_eq!(e.draw_range(5..6), 5);
    }
}

#[test]
fn draw_below_boundaries() {
    let mut e = SeededEntropy::from_seed(&seed());
    // 0/den is never true; den/den is always true — no draws can change that.
    for _ in 0..100 {
        assert!(!e.draw_below(0, 10));
    }
    for _ in 0..100 {
        assert!(e.draw_below(10, 10));
    }
}

#[test]
fn shuffle_len_is_a_permutation() {
    let mut e = SeededEntropy::from_seed(&seed());
    let perm = e.shuffle_len(8);
    let mut sorted = perm.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..8).collect::<Vec<_>>());
}

#[test]
fn shuffle_matches_shuffle_len() {
    // Two fresh streams at the same position: one returns the permutation, the
    // other shuffles the identity vector. They must agree.
    let permutation = SeededEntropy::from_seed(&seed()).shuffle_len(10);

    let mut shuffled: Vec<usize> = (0..10).collect();
    SeededEntropy::from_seed(&seed()).shuffle(&mut shuffled);

    assert_eq!(permutation, shuffled);
}

#[test]
fn shuffle_len_issues_len_minus_one_draws() {
    let mut e = SeededEntropy::from_seed(&seed());
    let start = e.draws();
    let _ = e.shuffle_len(1); // zero draws
    assert_eq!(e.draws(), start);
}

#[test]
fn golden_stream_vectors() {
    // Frozen: the first words of the ChaCha12 stream for seed [7; 32]. These pin
    // the engine's output across platforms; a mismatch is upstream drift, not a
    // value to regenerate. Run `emit_golden_stream --ignored --nocapture` to see
    // freshly-computed values when intentionally rebaselining.
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
