//! Index builds that run without the writer or tier locks: searches keep using the current tier
//! meanwhile, and tier changes made during the build are logged and replayed before the swap.

use std::sync::atomic::Ordering;

use super::super::{SqliteStore, poisoned};
use super::load;
use crate::domain::ChunkId;
use crate::error::{Error, Result};
use crate::vectors::Tier;
use crate::vectors::turbo::Turbo;

/// A tier change made while an index build runs.
pub(super) enum Change {
    /// A committed embedding to add to the new index.
    Add(ChunkId, Vec<f32>),
    /// A deleted chunk to drop from the new index.
    Remove(ChunkId),
}

/// The running index build, if any: its id and the changes logged since it started. Ids tell
/// a build cancelled by [`SqliteStore::reload_tier`] apart from one started after it.
#[derive(Default)]
pub struct Build {
    id: u64,
    log: Option<Vec<Change>>,
}

impl Build {
    /// Logs `change()` if a build runs; returns whether one does.
    pub(super) fn record(&mut self, change: impl FnOnce() -> Change) -> bool {
        self.log.as_mut().map(|log| log.push(change())).is_some()
    }

    /// Marks a new build as running and returns its id.
    pub(super) fn start(&mut self) -> u64 {
        self.id += 1;
        self.log = Some(Vec::new());
        self.id
    }

    /// Drops the running build's log; its result will be discarded.
    pub(super) fn cancel(&mut self) {
        self.log = None;
    }

    /// The log of build `id`, or `None` if that build was cancelled or replaced.
    fn log(&mut self, id: u64) -> Option<&mut Vec<Change>> {
        self.log.as_mut().filter(|_| self.id == id)
    }
}

/// Applies logged changes to a freshly built index, in log order.
///
/// # Errors
/// * [`Error::Storage`] - the index rejects a logged vector.
fn replay(index: &mut Turbo, changes: Vec<Change>) -> Result<()> {
    for change in changes {
        match change {
            Change::Add(id, vector) => index.add(&vector, &[id])?,
            Change::Remove(id) => index.remove(id),
        }
    }
    index.prepare();
    Ok(())
}

impl SqliteStore {
    /// Runs build `id` on a pooled read connection, holding neither the writer nor the tier
    /// lock, then replays the changes logged meanwhile and swaps it in under a short tier write
    /// lock. A failed build is logged, not returned: exact scans keep serving, and the build is
    /// retried once the count doubles. Does nothing if the build was cancelled meanwhile.
    ///
    /// # Arguments
    /// * `id` - the build id returned by [`Self::remember`].
    ///
    /// # Errors
    /// * [`Error::Storage`] - a tier lock is poisoned.
    pub(crate) fn build_index(&self, id: u64) -> Result<()> {
        let mut fresh = match self.read(|c| load(c, self.exact_max)) {
            Ok(Tier::Turbo(index)) => Some(index),
            Ok(Tier::Exact(_)) => None,
            Err(err) => return self.back_off(id, &err),
        };
        loop {
            let mut tier = self.tier.write().map_err(poisoned)?;
            let mut building = self.building.lock().map_err(poisoned)?;
            let Some(log) = building.log(id) else { return Ok(()) };
            let Some(index) = fresh.as_mut().filter(|_| !log.is_empty()) else {
                building.cancel();
                if let Some(index) = fresh {
                    *tier = Tier::Turbo(index);
                    self.build_at.store(0, Ordering::Relaxed);
                }
                return Ok(());
            };
            let changes = std::mem::take(log);
            drop((building, tier));
            if let Err(err) = replay(index, changes) {
                return self.back_off(id, &err);
            }
        }
    }

    /// Ends failed build `id`: stays on exact scan and retries only once the count doubles.
    ///
    /// # Errors
    /// * [`Error::Storage`] - a tier lock is poisoned.
    fn back_off(&self, id: u64, err: &Error) -> Result<()> {
        let tier = self.tier.write().map_err(poisoned)?;
        let mut building = self.building.lock().map_err(poisoned)?;
        if building.log(id).is_none() {
            return Ok(());
        }
        building.cancel();
        let n = if let Tier::Exact(n) = *tier { n } else { 0 };
        tracing::warn!(chunks = n, "vector index build failed, staying on exact scan: {err}");
        self.build_at.store(n.saturating_mul(2), Ordering::Relaxed);
        Ok(())
    }

    /// Removes chunks deleted by a committed write from the tier under its write lock (logging
    /// them for replay when an index build runs); an index shrunk to `exact_max / 2` rows or
    /// fewer falls back to exact scans.
    ///
    /// # Arguments
    /// * `ids` - ids of deleted chunks that had embeddings.
    ///
    /// # Errors
    /// * [`Error::Storage`] - a tier lock is poisoned.
    pub(crate) fn forget(&self, ids: &[ChunkId]) -> Result<()> {
        let mut tier = self.tier.write().map_err(poisoned)?;
        tier.remove(ids, self.exact_max);
        let mut building = self.building.lock().map_err(poisoned)?;
        for &id in ids {
            building.record(|| Change::Remove(id));
        }
        Ok(())
    }
}
