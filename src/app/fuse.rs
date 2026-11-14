//! Convex combination of min-max-normalised keyword and vector scores.

use std::collections::HashMap;

use crate::domain::NoteId;

/// Merges keyword and vector rankings into one: each list is min-max normalised to 0..=1 (a
/// single hit or all-equal scores normalise to 1.0, a note missing from a list gets 0 there),
/// then score = (1 - alpha) · keyword + alpha · vector. Pure.
///
/// # Arguments
/// * `keyword` - (note, score) pairs from full-text search, higher is better.
/// * `vector` - (note, cosine score) pairs from vector search, higher is better.
/// * `alpha` - Weight of the vector list, 0.0..=1.0; the keyword list gets `1 - alpha`.
///
/// # Returns
/// Every note found in either list, best fused score first; ties by ascending [`NoteId`].
pub fn fuse(keyword: &[(NoteId, f64)], vector: &[(NoteId, f64)], alpha: f64) -> Vec<NoteId> {
    let mut scores: HashMap<NoteId, f64> = HashMap::new();
    for (list, weight) in [(keyword, 1.0 - alpha), (vector, alpha)] {
        let lo = list.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
        let hi = list.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
        for &(id, s) in list {
            let norm = if hi > lo { (s - lo) / (hi - lo) } else { 1.0 };
            *scores.entry(id).or_default() += weight * norm;
        }
    }
    let mut ranked: Vec<(NoteId, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds (note, score) pairs from raw `(id, score)` tuples.
    ///
    /// # Arguments
    /// * `v` - Note ids with their scores.
    ///
    /// # Returns
    /// The same pairs with ids wrapped in [`NoteId`].
    fn scored(v: &[(i64, f64)]) -> Vec<(NoteId, f64)> {
        v.iter().map(|&(i, s)| (NoteId(i), s)).collect()
    }

    /// Wraps raw ids as [`NoteId`]s, to compare against `fuse` output.
    ///
    /// # Arguments
    /// * `v` - Note ids in expected order.
    ///
    /// # Returns
    /// The ids as a vector of [`NoteId`].
    fn ids(v: &[i64]) -> Vec<NoteId> {
        v.iter().map(|&i| NoteId(i)).collect()
    }

    /// Proves a wide keyword (BM25) lead survives a vector list whose top scores are nearly
    /// equal, by fusing hand-computed lists at `alpha = 0.5` and checking the order.
    #[test]
    fn bm25_margin_beats_flat_vector_list() {
        // Keyword: note 1 leads by a wide margin. Vector: the top three sit within 0.01 above a long tail.
        // Normalised: note 1 = 0.5·1.0 + 0.5·0.975, note 2 = 0.5·0.25 + 0.5·1.0, so the keyword winner stays first.
        let keyword = scored(&[(1, 9.0), (2, 3.0), (3, 2.5), (4, 1.0)]);
        let vector = scored(&[(2, 0.75), (3, 0.745), (1, 0.74), (4, 0.35)]);
        assert_eq!(fuse(&keyword, &vector, 0.5), ids(&[1, 2, 3, 4]));
    }

    /// Proves a lone keyword hit and an all-equal keyword list each normalise to 1.0, checked
    /// against orders computed by hand.
    #[test]
    fn single_hit_and_equal_scores_normalise_to_one() {
        // Note 3 is the only keyword hit: 0.5 · 1.0 ties note 1 (vector best) and beats note 2 (vector worst, 0).
        assert_eq!(fuse(&scored(&[(3, 5.0)]), &scored(&[(1, 0.9), (2, 0.1)]), 0.5), ids(&[1, 3, 2]));
        let equal = scored(&[(4, 2.0), (5, 2.0)]);
        assert_eq!(fuse(&equal, &scored(&[(1, 0.8), (2, 0.2)]), 0.5), ids(&[1, 4, 5, 2]));
    }

    /// Proves that with one list empty the other list's order (ties by ascending id) is kept,
    /// and that two empty lists give no result.
    #[test]
    fn one_list_keeps_its_order_and_empty_is_empty() {
        assert_eq!(fuse(&[], &scored(&[(7, 0.2), (5, 0.9)]), 0.5), ids(&[5, 7]));
        assert_eq!(fuse(&scored(&[(9, 1.0), (2, 1.0)]), &[], 0.5), ids(&[2, 9]));
        assert!(fuse(&[], &[], 0.5).is_empty());
    }

    /// Proves a note found only by keyword with the top score ties a note found by both lists
    /// with weak scores, the tie going to the lower id, using hand-computed fused scores.
    #[test]
    fn strong_keyword_only_beats_weak_hit_in_both_lists() {
        // Keyword 10,2,0 → 1,0.2,0. Vector 0.7,0.9,0.5 → 0.5,1,0. Missing lists count 0.
        // Note 5: 0.5·1 = 0.5; note 3: 0.5·0 + 0.5·1 = 0.5; note 2: 0.5·0.2 + 0.5·0.5 = 0.35; note 4: 0.
        // Notes 5 and 3 tie, so the lower ID goes first.
        let keyword = scored(&[(5, 10.0), (2, 2.0), (3, 0.0)]);
        let vector = scored(&[(2, 0.7), (3, 0.9), (4, 0.5)]);
        assert_eq!(fuse(&keyword, &vector, 0.5), ids(&[3, 5, 2, 4]));
    }
}
