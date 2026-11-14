//! Vector tier chosen by embedded-chunk count: exact scan, or turbovec candidates.

pub mod turbo;

use crate::domain::ChunkId;
use crate::error::Result;

/// Search path for stored embeddings.
pub enum Tier {
    /// Few enough vectors to scan them all; holds the embedded-chunk count.
    Exact(usize),
    /// A 4-bit index proposing candidates for exact re-ranking.
    Turbo(Box<turbo::Turbo>),
}

impl Tier {
    /// Counts one newly stored embedding, or adds it to the 4-bit index when that tier is active.
    ///
    /// # Arguments
    /// * `id` - Chunk the embedding belongs to.
    /// * `vector` - The embedding; used only by the indexed tier.
    /// * `build_at` - Largest chunk count the exact-scan tier should hold before an index build;
    ///   `exact_max`, or higher while backing off from a failed build.
    ///
    /// # Returns
    /// `true` once the exact tier holds more than `build_at` chunks, so the caller should build
    /// the index; always `false` for the indexed tier.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - Indexed tier only: the index rejects the vector
    ///   (wrong width, non-finite value or negative id). The exact tier never fails.
    pub fn insert(&mut self, id: ChunkId, vector: &[f32], build_at: usize) -> Result<bool> {
        match self {
            Self::Turbo(index) => index.add(vector, &[id]).map(|()| false),
            Self::Exact(n) => {
                *n += 1;
                Ok(*n > build_at)
            }
        }
    }

    /// Forgets removed chunks; the indexed tier falls back to exact scan once it holds at most
    /// `exact_max / 2` rows, so a count hovering near `exact_max` does not rebuild the index on
    /// every write.
    ///
    /// # Arguments
    /// * `ids` - Chunks whose embeddings were deleted.
    /// * `exact_max` - Largest chunk count the exact-scan tier should hold.
    pub fn remove(&mut self, ids: &[ChunkId], exact_max: usize) {
        match self {
            Self::Exact(n) => *n = n.saturating_sub(ids.len()),
            Self::Turbo(index) => {
                ids.iter().for_each(|&id| index.remove(id));
                let n = index.len();
                if n <= exact_max / 2 {
                    *self = Self::Exact(n);
                }
            }
        }
    }

    /// Proposes candidate chunk ids for `query`, best first, for exact re-ranking.
    ///
    /// # Arguments
    /// * `query` - Query embedding.
    ///
    /// # Returns
    /// `None` in the exact tier, meaning every stored vector should be scanned instead.
    ///
    /// # Errors
    /// * [`crate::error::Error::Storage`] - `query` has the wrong width or a non-finite value.
    pub fn candidates(&self, query: &[f32]) -> Result<Option<Vec<ChunkId>>> {
        match self {
            Self::Exact(_) => Ok(None),
            Self::Turbo(index) => index.candidates(query).map(Some),
        }
    }
}
