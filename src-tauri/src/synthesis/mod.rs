//! Sigil Synthesis prediction engine.
//!
//! Pure port of the game's synthesis algorithm (v2.0.2, reverse-engineered —
//! see docs/superpowers/specs/2026-07-18-synthesis-helper-design.md). The
//! snapshot module reads the inputs from game memory; everything here is
//! deterministic and unit-testable.

pub mod snapshot;

use serde::Serialize;
use std::collections::HashMap;

/// The game's "no trait in this slot" sentinel.
pub const EMPTY_TRAIT: u32 = 0x887a_e0b0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisSigil {
    /// Per-copy instance uid (map key in the sigil manager).
    pub uid: u32,
    /// The sigil's item id (GEEN_* hash) — translatable via `sigils.json`.
    pub sigil_id: u32,
    pub trait1: u32,
    pub trait1_level: u32,
    pub trait2: u32,
    pub trait2_level: u32,
    /// `record+8` of the sigil's item-config record; feeds the warm-up count.
    #[serde(skip)]
    pub record_level: i32,
}

#[derive(Debug, Default)]
pub struct SynthesisSnapshot {
    /// xorshift32 state of RNG slot 0x81 at snapshot time.
    pub rng_state: u32,
    /// MGR+0x2d8; part of the warm-up count.
    pub seed_counter: u32,
    /// pairKey -> times this pair-shape has been synthesized.
    pub pair_counters: HashMap<u64, u32>,
    /// rank(A)+rank(B) -> (lo, hi) level-roll weights.
    pub level_weights: HashMap<u32, (u32, u32)>,
    /// first result trait -> result sigil item id.
    pub trait_to_item: HashMap<u32, u32>,
    pub sigils: Vec<SynthesisSigil>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prediction {
    pub trait1: u32,
    pub trait2: Option<u32>,
    /// true = the weighted roll hit the upgraded (level-15) outcome.
    pub lucky: bool,
}

/// One step of the game's per-slot RNG. Returns the new state, which is also
/// the drawn value.
#[inline]
pub fn xorshift32(mut s: u32) -> u32 {
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 15;
    s
}

fn trait_sum(s: &SynthesisSigil) -> u64 {
    let t1 = if s.trait1 == EMPTY_TRAIT { 0 } else { s.trait1 as u64 };
    let t2 = if s.trait2 == EMPTY_TRAIT { 0 } else { s.trait2 as u64 };
    t1 + t2
}

fn rank(s: &SynthesisSigil) -> u32 {
    let l1 = if s.trait1 == EMPTY_TRAIT { 0 } else { s.trait1_level };
    let l2 = if s.trait2 == EMPTY_TRAIT { 0 } else { s.trait2_level };
    l1.wrapping_add(l2)
}

pub fn predict(snap: &SynthesisSnapshot, a: &SynthesisSigil, b: &SynthesisSigil) -> Prediction {
    let pair_key = trait_sum(a)
        + trait_sum(b)
        + (a.record_level.wrapping_add(b.record_level) as u32) as u64;
    let n = snap.pair_counters.get(&pair_key).copied().unwrap_or(0).wrapping_add(1);
    let warm = (n.wrapping_mul(9) as u64)
        .wrapping_add(pair_key)
        .wrapping_add(snap.seed_counter as u64)
        % 1000;

    let mut s = snap.rng_state;
    for _ in 0..warm {
        s = xorshift32(s);
    }

    let (lo, hi) = snap
        .level_weights
        .get(&rank(a).wrapping_add(rank(b)))
        .copied()
        .unwrap_or((0, 0));
    s = xorshift32(s); // the level roll always draws, even with no weights
    let lucky = lo + hi > 0 && (s % (lo + hi)) >= lo;

    let mut cand: Vec<u32> = [a.trait1, a.trait2, b.trait1, b.trait2]
        .into_iter()
        .filter(|&t| t != EMPTY_TRAIT)
        .collect();
    cand.sort_unstable();
    let len = cand.len();
    for i in 0..len {
        s = xorshift32(s);
        let rem = (len - i) as u32;
        let mut r = s;
        if r >= rem {
            r %= rem;
        }
        cand.swap(i, i + r as usize);
    }

    Prediction {
        trait1: cand[0],
        trait2: cand.get(1).copied(),
        lucky,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sigil(uid: u32, t1: u32, l1: u32, t2: u32, l2: u32, rec: i32) -> SynthesisSigil {
        SynthesisSigil {
            uid,
            sigil_id: 0,
            trait1: t1,
            trait1_level: l1,
            trait2: t2,
            trait2_level: l2,
            record_level: rec,
        }
    }

    /// Reference sequence computed independently from the decompiled
    /// algorithm: s=1 -> 0x1000a001, 0x45000201, 0x451080a1, 0x10150a23, ...
    #[test]
    fn xorshift32_reference_sequence() {
        let mut s = 1u32;
        let expect = [0x1000a001u32, 0x45000201, 0x451080a1, 0x10150a23, 0x2814b28b];
        for e in expect {
            s = xorshift32(s);
            assert_eq!(s, e);
        }
    }

    /// Full predict() against an independently computed fixture:
    /// A = {t 0x100 l11, t 0x200 l15, rec 5}, B = {t 0x300 l12, empty, rec 7}
    /// pair_key = 0x100+0x200+0x300+(5+7) = 1548; counters empty -> n=1;
    /// seed_counter = 42 -> warm = (9+1548+42) % 1000 = 599.
    /// rng_state = 123456789; weights {38: (3,7)}.
    /// Expected (Python reference): lucky = true, result = [0x300, 0x200] (0x100 last).
    #[test]
    fn predict_reference_fixture() {
        let a = sigil(0xA, 0x100, 11, 0x200, 15, 5);
        let b = sigil(0xB, 0x300, 12, EMPTY_TRAIT, 0, 7);
        let mut snap = SynthesisSnapshot {
            rng_state: 123_456_789,
            seed_counter: 42,
            ..Default::default()
        };
        snap.level_weights.insert(38, (3, 7));
        let p = predict(&snap, &a, &b);
        assert_eq!(p.trait1, 0x300);
        assert_eq!(p.trait2, Some(0x200));
        assert!(p.lucky);
    }

    /// The algorithm only sums the two sigils' contributions — order must not matter.
    #[test]
    fn predict_is_symmetric() {
        let a = sigil(0xA, 0x100, 11, 0x200, 15, 5);
        let b = sigil(0xB, 0x300, 12, 0x400, 3, 7);
        let mut snap = SynthesisSnapshot {
            rng_state: 0xdead_beef,
            seed_counter: 7,
            ..Default::default()
        };
        snap.level_weights.insert(41, (10, 1));
        assert_eq!(predict(&snap, &a, &b), predict(&snap, &b, &a));
    }

    /// Missing weight entry (or lo+hi == 0) can never be lucky, but the level
    /// draw still advances the stream before the shuffle.
    #[test]
    fn predict_no_weights_is_never_lucky() {
        let a = sigil(0xA, 0x100, 11, 0x200, 15, 5);
        let b = sigil(0xB, 0x300, 12, EMPTY_TRAIT, 0, 7);
        let snap = SynthesisSnapshot {
            rng_state: 123_456_789,
            seed_counter: 42,
            ..Default::default()
        };
        let p = predict(&snap, &a, &b);
        assert!(!p.lucky);
        // Same draws as the reference fixture -> same shuffle outcome.
        assert_eq!(p.trait1, 0x300);
        assert_eq!(p.trait2, Some(0x200));
    }

    /// A pair counter shifts the warm-up by 9 per prior synthesis.
    #[test]
    fn predict_pair_counter_changes_warmup() {
        let a = sigil(0xA, 0x100, 11, 0x200, 15, 5);
        let b = sigil(0xB, 0x300, 12, EMPTY_TRAIT, 0, 7);
        let mut snap = SynthesisSnapshot {
            rng_state: 123_456_789,
            seed_counter: 42,
            ..Default::default()
        };
        let base = predict(&snap, &a, &b);
        snap.pair_counters.insert(1548, 3); // n becomes 4 -> warm = 626 instead of 599
        let shifted = predict(&snap, &a, &b);
        assert_ne!(base, shifted);
        // Python reference for warm=626: result [0x200, 0x300, 0x100]
        assert_eq!(shifted.trait1, 0x200);
        assert_eq!(shifted.trait2, Some(0x300));
    }
}
