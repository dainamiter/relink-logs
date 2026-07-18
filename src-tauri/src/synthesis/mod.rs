//! Sigil Synthesis prediction engine.
//!
//! Pure port of the game's synthesis algorithm (v2.0.2, reverse-engineered —
//! see docs/superpowers/specs/2026-07-18-synthesis-helper-design.md). The
//! snapshot module reads the inputs from game memory; everything here is
//! deterministic and unit-testable.

pub mod snapshot;

use serde::{Deserialize, Serialize};
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

/// Predict the result of synthesizing `a` + `b` under `snap`'s RNG state.
///
/// Precondition: at least one of the four trait slots is non-empty (every
/// real sigil has a first trait). Two fully-blank sigils would make the
/// candidate list empty.
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
    let weight_total = lo.wrapping_add(hi);
    let lucky = weight_total > 0 && (s % weight_total) >= lo;

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

    debug_assert!(!cand.is_empty(), "predict() called with two traitless sigils");

    Prediction {
        trait1: cand[0],
        trait2: cand.get(1).copied(),
        lucky,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisQuery {
    pub trait1: u32,
    pub trait2: Option<u32>,
    pub any_order: bool,
    pub require_lucky: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisMatch {
    pub sigil_a: SynthesisSigil,
    pub sigil_b: SynthesisSigil,
    pub prediction: Prediction,
    /// Item id of the result sigil (for display), when known.
    pub result_sigil_id: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisStatus {
    pub game_running: bool,
    pub sigil_count: u32,
    /// True when RNG state is 0 (the game will reseed from entropy — unpredictable).
    pub rng_unpredictable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisSearchResponse {
    pub matches: Vec<SynthesisMatch>,
    pub pairs_tested: u64,
    pub total_matches: u64,
    pub sigil_count: u32,
    pub rng_unpredictable: bool,
}

/// Test every unordered pair whose combined traits could contain the queried
/// ones; return (matches up to `cap`, pairs actually predicted, total matches).
pub fn search(
    snap: &SynthesisSnapshot,
    q: &SynthesisQuery,
    cap: usize,
) -> (Vec<SynthesisMatch>, u64, u64) {
    let has = |s: &SynthesisSigil, t: u32| s.trait1 == t || s.trait2 == t;
    let wanted = |p: &Prediction| -> bool {
        if q.require_lucky && !p.lucky {
            return false;
        }
        match q.trait2 {
            None => p.trait1 == q.trait1 || (q.any_order && p.trait2 == Some(q.trait1)),
            Some(t2) => {
                let exact = p.trait1 == q.trait1 && p.trait2 == Some(t2);
                let swapped = p.trait1 == t2 && p.trait2 == Some(q.trait1);
                exact || (q.any_order && swapped)
            }
        }
    };

    let mut matches = Vec::new();
    let (mut tested, mut total) = (0u64, 0u64);
    for i in 0..snap.sigils.len() {
        for j in (i + 1)..snap.sigils.len() {
            let (a, b) = (&snap.sigils[i], &snap.sigils[j]);
            if !has(a, q.trait1) && !has(b, q.trait1) {
                continue;
            }
            if let Some(t2) = q.trait2 {
                if !has(a, t2) && !has(b, t2) {
                    continue;
                }
            }
            tested += 1;
            let p = predict(snap, a, b);
            if wanted(&p) {
                total += 1;
                if matches.len() < cap {
                    matches.push(SynthesisMatch {
                        sigil_a: a.clone(),
                        sigil_b: b.clone(),
                        prediction: p,
                        result_sigil_id: snap.trait_to_item.get(&p.trait1).copied(),
                    });
                }
            }
        }
    }
    (matches, tested, total)
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

    fn search_snap() -> SynthesisSnapshot {
        let mut snap = SynthesisSnapshot {
            rng_state: 123_456_789,
            seed_counter: 42,
            ..Default::default()
        };
        snap.trait_to_item.insert(0x300, 0x9999);
        snap.sigils = vec![
            sigil(1, 0x100, 11, 0x200, 15, 5),
            sigil(2, 0x300, 12, EMPTY_TRAIT, 0, 7),
            sigil(3, 0x500, 10, 0x600, 10, 4),
        ];
        snap
    }

    /// Pair (1,2) is the reference fixture -> predicts (0x300, 0x200).
    #[test]
    fn search_finds_matching_pair() {
        let snap = search_snap();
        let q = SynthesisQuery {
            trait1: 0x300,
            trait2: Some(0x200),
            any_order: false,
            require_lucky: false,
        };
        let (matches, tested, total) = search(&snap, &q, 100);
        assert_eq!(total, 1);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].sigil_a.uid, 1);
        assert_eq!(matches[0].sigil_b.uid, 2);
        assert_eq!(matches[0].result_sigil_id, Some(0x9999));
        // pairs (1,3) and (2,3) can't produce 0x300+0x200 together; only (1,2) is tested
        assert_eq!(tested, 1);
    }

    /// Exact order excludes the swapped outcome; any_order accepts it.
    #[test]
    fn search_order_toggle() {
        let snap = search_snap();
        let exact = SynthesisQuery { trait1: 0x200, trait2: Some(0x300), any_order: false, require_lucky: false };
        let (m, _, total) = search(&snap, &exact, 100);
        assert_eq!(total, 0);
        assert!(m.is_empty());
        let any = SynthesisQuery { trait1: 0x200, trait2: Some(0x300), any_order: true, require_lucky: false };
        let (m, _, total) = search(&snap, &any, 100);
        assert_eq!(total, 1);
        assert_eq!(m.len(), 1);
    }

    /// require_lucky filters out normal rolls (fixture has no weights -> never lucky).
    #[test]
    fn search_require_lucky() {
        let snap = search_snap();
        let q = SynthesisQuery { trait1: 0x300, trait2: Some(0x200), any_order: false, require_lucky: true };
        let (m, _, total) = search(&snap, &q, 100);
        assert_eq!(total, 0);
        assert!(m.is_empty());
    }

    /// Reference vector for the 4-candidate shuffle with a DUPLICATED trait
    /// (duplicates must be kept): A = {0x100 l5, 0x300 l10, rec 2},
    /// B = {0x300 l12, 0x400 l1, rec 3}; pair_key = 2821, seed 100 -> warm 930;
    /// rng 987654321; ranksum 28, weights {28: (2,5)}.
    /// Python reference: lucky = true, shuffled = [0x400, 0x300, 0x300, 0x100].
    #[test]
    fn predict_four_candidates_with_duplicate() {
        let a = sigil(0xA, 0x100, 5, 0x300, 10, 2);
        let b = sigil(0xB, 0x300, 12, 0x400, 1, 3);
        let mut snap = SynthesisSnapshot {
            rng_state: 987_654_321,
            seed_counter: 100,
            ..Default::default()
        };
        snap.level_weights.insert(28, (2, 5));
        let p = predict(&snap, &a, &b);
        assert_eq!(p.trait1, 0x400);
        assert_eq!(p.trait2, Some(0x300));
        assert!(p.lucky);
    }

    /// cap bounds the returned matches but `total` keeps counting.
    #[test]
    fn search_cap_truncates_matches_not_total() {
        // Three sigils all carrying trait 0x100 -> pairs (1,2),(1,3),(2,3) all
        // feed predict(); with trait2=None the query matches any pair whose
        // result leads with 0x100. Pick a query that matches >= 2 of them.
        let mut snap = SynthesisSnapshot {
            rng_state: 555,
            seed_counter: 1,
            ..Default::default()
        };
        snap.sigils = vec![
            sigil(1, 0x100, 10, EMPTY_TRAIT, 0, 1),
            sigil(2, 0x100, 10, EMPTY_TRAIT, 0, 1),
            sigil(3, 0x100, 10, EMPTY_TRAIT, 0, 1),
        ];
        // Every pair's only candidate is 0x100 -> trait1 is always 0x100.
        let q = SynthesisQuery { trait1: 0x100, trait2: None, any_order: false, require_lucky: false };
        let (all, tested, total) = search(&snap, &q, 100);
        assert_eq!(tested, 3);
        assert_eq!(total, 3);
        assert_eq!(all.len(), 3);
        // Now cap at 1: still 3 tested / 3 total, but only 1 returned.
        let (capped, tested, total) = search(&snap, &q, 1);
        assert_eq!(tested, 3);
        assert_eq!(total, 3);
        assert_eq!(capped.len(), 1);
    }
}
