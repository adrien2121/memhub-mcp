//! turbovec 4-bit candidate search; callers re-rank candidates with exact vectors.

use turbovec::IdMapIndex;

use crate::domain::ChunkId;
use crate::error::{Error, Result};

/// Indexed rows per candidate; Recall@10 ≥ 0.98 was measured with this ratio.
const ROWS_PER_CANDIDATE: usize = 250;
/// Fewest candidates returned by a search: four per note of the 100-note vector list that search fuses.
const MIN_CANDIDATES: usize = 400;

/// 4-bit turbovec index keyed by chunk id.
pub struct Turbo {
    index: IdMapIndex,
}

/// Wraps a turbovec index failure as a storage error so callers see one error type.
///
/// # Arguments
/// * `e` - The turbovec error, or any displayable failure from the index.
///
/// # Returns
/// [`Error::Storage`] whose message is prefixed with `vector index: `.
fn index_err(e: impl std::fmt::Display) -> Error {
    Error::Storage(format!("vector index: {e}"))
}

/// Converts a chunk id to the unsigned key turbovec stores rows under.
///
/// # Arguments
/// * `id` - Chunk id; negative ids cannot be represented as turbovec's unsigned keys.
///
/// # Returns
/// The id as `u64`.
///
/// # Errors
/// * [`Error::Storage`] - `id` is negative.
fn key(id: ChunkId) -> Result<u64> {
    u64::try_from(id.0).map_err(|_| Error::Storage(format!("negative chunk id {}", id.0)))
}

impl Turbo {
    /// Creates an empty 4-bit index and calibrates its quantiser on sample vectors.
    ///
    /// # Arguments
    /// * `dim` - Vector width (number of `f32` values per row).
    /// * `sample` - Row-major sample vectors, `dim` values per row, used for calibration.
    ///
    /// # Returns
    /// The calibrated, empty index.
    ///
    /// # Errors
    /// * [`Error::Storage`] - `dim` is invalid, or the sample is too small or degenerate.
    pub fn new(dim: usize, sample: &[f32]) -> Result<Self> {
        let mut index = IdMapIndex::new(dim, 4).map_err(index_err)?;
        index.calibrate(sample).map_err(index_err)?;
        Ok(Self { index })
    }

    /// Adds vectors under chunk ids, first removing any rows already stored under those ids.
    ///
    /// # Arguments
    /// * `vectors` - Row-major vectors, one row of the index width per id.
    /// * `ids` - Chunk id of each row, in row order.
    ///
    /// # Errors
    /// * [`Error::Storage`] - An id is negative (nothing is changed), or the index rejects the
    ///   rows for a width mismatch or a non-finite value (the old rows are already removed).
    pub fn add(&mut self, vectors: &[f32], ids: &[ChunkId]) -> Result<()> {
        let keys = ids.iter().map(|&id| key(id)).collect::<Result<Vec<_>>>()?;
        for &k in &keys {
            self.index.remove(k);
        }
        self.index.add_with_ids(vectors, &keys).map_err(index_err)
    }

    /// Removes the row stored under a chunk id; does nothing if it is absent or negative.
    ///
    /// # Arguments
    /// * `id` - Chunk id to drop.
    pub fn remove(&mut self, id: ChunkId) {
        if let Ok(k) = key(id) {
            self.index.remove(k);
        }
    }

    /// Counts the indexed rows.
    ///
    /// # Returns
    /// Number of rows currently in the index.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Tells whether the index holds no rows.
    ///
    /// # Returns
    /// `true` if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Builds the index's search caches now, so the first query does not pay for them.
    pub fn prepare(&self) {
        self.index.prepare();
    }

    /// Finds the chunk ids of the rows closest to `query`, best first, for exact re-ranking.
    ///
    /// Asks for `len / 250` rows (the `ROWS_PER_CANDIDATE` constant), but never fewer than 400
    /// (the `MIN_CANDIDATES` constant); turbovec may return fewer when the index is smaller.
    ///
    /// # Arguments
    /// * `query` - Query vector of the index width.
    ///
    /// # Returns
    /// Candidate chunk ids, closest first.
    ///
    /// # Errors
    /// * [`Error::Storage`] - `query` has the wrong width or a non-finite value, or a returned
    ///   key does not fit in a chunk id.
    pub fn candidates(&self, query: &[f32]) -> Result<Vec<ChunkId>> {
        let k = (self.index.len() / ROWS_PER_CANDIDATE).max(MIN_CANDIDATES);
        let found = self.index.try_search(query, k).map_err(index_err)?;
        found.ids.iter().map(|&id| i64::try_from(id).map(ChunkId).map_err(index_err)).collect()
    }
}
