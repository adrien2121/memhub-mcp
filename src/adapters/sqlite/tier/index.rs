//! The approximate-nearest-neighbour index: building it over every stored embedding of live
//! notes, and reading back the live chunks it returns as candidates.

use rusqlite::Connection;

use super::super::{chunks, storage};
use crate::domain::{ChunkId, NoteId};
use crate::error::{Error, Result};
use crate::vectors::turbo::Turbo;

/// Rows streamed into the index per batch while building.
const BATCH: usize = 4096;
/// Target number of rows sampled to calibrate the index before rows are added.
const SAMPLE: usize = 10_000;

/// Streams the embedded chunks of live notes whose chunk id is a multiple of `stride`, decoding
/// each embedding into one reused buffer.
///
/// # Arguments
/// * `conn` - any connection to the store.
/// * `stride` - keep only chunk ids divisible by this; `1` streams every embedded chunk.
/// * `visit` - called with the chunk id and its embedding; the slice is only valid during the
///   call. An error from it stops the scan.
///
/// # Errors
/// * [`Error::Storage`] - `stride` exceeds `i64::MAX`, the query fails, or a blob is malformed.
/// * Any error returned by `visit`.
fn scan(conn: &Connection, stride: usize, visit: &mut dyn FnMut(ChunkId, &[f32]) -> Result<()>) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.id, c.embedding FROM chunks c JOIN notes n ON n.id = c.note_id \
             WHERE c.embedding IS NOT NULL AND n.deleted_at IS NULL AND c.id % ?1 = 0",
        )
        .map_err(storage)?;
    let stride = i64::try_from(stride).map_err(|_| Error::Storage(format!("bad stride {stride}")))?;
    let mut rows = stmt.query([stride]).map_err(storage)?;
    let mut buf = Vec::new();
    while let Some(r) = rows.next().map_err(storage)? {
        let id = ChunkId(r.get(0).map_err(storage)?);
        chunks::decode(r.get_ref(1).map_err(storage)?, id, &mut buf)?;
        visit(id, &buf)?;
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// Test seam: build attempts on this thread, and whether the next attempts should fail.
    pub(super) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test seam: run once the rows are scanned, before the index is returned.
    pub(super) static DURING: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

/// Builds the index: first calibrates it on roughly 10,000 rows ([`SAMPLE`]) picked by chunk-id stride,
/// then makes a second pass adding every row in batches of 4,096 ([`BATCH`]), so memory stays bounded.
///
/// # Arguments
/// * `conn` - any connection to the store.
/// * `n` - number of stored embeddings; sets the sampling stride.
///
/// # Returns
/// The built and prepared index.
///
/// # Errors
/// * [`Error::Storage`] - a query fails, a blob is malformed, or the index rejects the sample or
///   a batch (for example on mixed vector widths).
pub(super) fn build(conn: &Connection, n: usize) -> Result<Turbo> {
    #[cfg(test)]
    {
        BUILDS.set(BUILDS.get() + 1);
        if FAIL.get() {
            return Err(crate::error::Error::Storage("injected build failure".into()));
        }
    }
    let (mut sample, mut dim) = (Vec::new(), 0);
    scan(conn, n.div_ceil(SAMPLE).max(1), &mut |_, v| {
        dim = v.len();
        sample.extend_from_slice(v);
        Ok(())
    })?;
    let mut index = Turbo::new(dim, &sample)?;
    drop(sample);
    let (mut ids, mut rows) = (Vec::with_capacity(BATCH), Vec::with_capacity(BATCH * dim));
    scan(conn, 1, &mut |id, v| {
        ids.push(id);
        rows.extend_from_slice(v);
        if ids.len() == BATCH {
            index.add(&rows, &ids)?;
            ids.clear();
            rows.clear();
        }
        Ok(())
    })?;
    if !ids.is_empty() {
        index.add(&rows, &ids)?;
    }
    #[cfg(test)]
    DURING.with_borrow_mut(|f| f.as_mut().map(|f| f()));
    index.prepare();
    Ok(index)
}

/// Calls `visit` for each of `ids` that is still embedded in a live note, in `ids` order;
/// ids deleted since the index returned them are skipped.
///
/// # Arguments
/// * `conn` - a pooled read connection.
/// * `ids` - candidate chunk ids from the index, best first.
/// * `visit` - called with chunk id, note id and the stored embedding; the slice is only valid
///   during the call.
///
/// # Errors
/// * [`Error::Storage`] - the query fails or a stored blob is malformed.
pub(super) fn visit_live(
    conn: &Connection,
    ids: &[ChunkId],
    visit: &mut dyn FnMut(ChunkId, NoteId, &[f32]),
) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.note_id, c.embedding FROM chunks c JOIN notes n ON n.id = c.note_id \
             WHERE c.id = ?1 AND c.embedding IS NOT NULL AND n.deleted_at IS NULL",
        )
        .map_err(storage)?;
    let mut buf = Vec::new();
    for &id in ids {
        let mut rows = stmt.query([id.0]).map_err(storage)?;
        if let Some(r) = rows.next().map_err(storage)? {
            let note = NoteId(r.get(0).map_err(storage)?);
            chunks::decode(r.get_ref(1).map_err(storage)?, id, &mut buf)?;
            visit(id, note, &buf);
        }
    }
    Ok(())
}
