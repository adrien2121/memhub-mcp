//! Cosine ranking over the store's nearest candidates, keeping each note's best chunk.

use std::collections::HashMap;

use crate::domain::{ChunkId, NoteId, SearchFilter};
use crate::error::Result;
use crate::ports::store::NoteStore;

/// Scores every stored chunk embedding the store offers for `filter` against `query` and ranks
/// notes by their best chunk. Vectors are L2-normalised by `app::embedding`, so cosine
/// similarity is the dot product. Read-only.
///
/// # Arguments
/// * `store` - Source of chunk embeddings, visited through `for_each_near`.
/// * `filter` - Tag and update-time restrictions on which notes may match.
/// * `query` - Unit-length query embedding, same dimension as the stored embeddings.
/// * `depth` - Most notes to return.
///
/// # Returns
/// Up to `depth` tuples of (note, cosine score in -1..=1, id of the note's best-scoring chunk),
/// best first; ties by ascending [`NoteId`].
///
/// # Errors
/// * [`Error::Storage`](crate::error::Error::Storage) - The store fails while reading embeddings.
pub fn nearest(
    store: &dyn NoteStore,
    filter: &SearchFilter,
    query: &[f32],
    depth: usize,
) -> Result<Vec<(NoteId, f64, ChunkId)>> {
    let mut best: HashMap<NoteId, (f32, ChunkId)> = HashMap::new();
    store.for_each_near(filter, query, &mut |chunk, note, v| {
        let score: f32 = v.iter().zip(query).map(|(a, b)| a * b).sum();
        let entry = best.entry(note).or_insert((f32::NEG_INFINITY, chunk));
        if score > entry.0 {
            *entry = (score, chunk);
        }
    })?;
    let mut ranked: Vec<(NoteId, f32, ChunkId)> = best.into_iter().map(|(n, (s, c))| (n, s, c)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(ranked.into_iter().take(depth).map(|(n, s, c)| (n, f64::from(s), c)).collect())
}
