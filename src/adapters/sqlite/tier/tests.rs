//! Index build failure: saves stay Ok, exact scan serves, builds back off.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::index::{BUILDS, DURING, FAIL};
use super::*;
use crate::domain::{NewChunk, NewNote};
use crate::ports::store::{EmbeddingQueue, NoteRecords, NoteSearch};

/// Stores one embedded single-chunk note and returns its chunk id.
fn embed_one(s: &SqliteStore, i: usize) -> ChunkId {
    let note = NewNote { title: format!("n{i}"), body: "x".into(), tags: vec![] };
    s.create(&note, &[NewChunk { seq: 0, start: 0, end: 1, text: "x".into() }], false).unwrap();
    let chunk = s.pending_chunks(ChunkId(0), 1).unwrap()[0].id;
    let mut v = vec![0.1_f32; 8];
    v[i % 8] = 1.0;
    assert!(s.set_embedding(chunk, &v).expect("committed write must not error"));
    chunk
}

#[test]
fn failed_index_build_keeps_exact_scan_and_backs_off() {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_tier_fail_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = SqliteStore::open_with_tokenizer(&dir.join("m.db"), "porter unicode61")
        .unwrap()
        .with_exact_max_chunks(2)
        .unwrap();
    embed_one(&s, 0);
    embed_one(&s, 1);
    BUILDS.set(0);
    FAIL.set(true);
    /// Clears the injected failure even if an assertion panics.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL.set(false);
        }
    }
    let _reset = Reset;

    // Count 3 passes exact_max: the build fails, yet the save succeeds and persists.
    let third = embed_one(&s, 2);
    assert_eq!(BUILDS.get(), 1);
    let stored: bool = s
        .read(|c| {
            c.query_row("SELECT embedding IS NOT NULL FROM chunks WHERE id = ?1", [third.0], |r| r.get(0))
                .map_err(storage)
        })
        .unwrap();
    assert!(stored);
    let mut seen = Vec::new();
    s.for_each_near(&SearchFilter::default(), &[0.1; 8], &mut |c, _, _| seen.push(c)).unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen.contains(&third));

    // Later saves up to twice the failed count do not rebuild.
    for i in 3..6 {
        embed_one(&s, i);
    }
    assert_eq!(BUILDS.get(), 1);
    assert!(matches!(*s.tier.read().unwrap(), Tier::Exact(6)));

    // Passing it retries once; a healthy build switches to the index.
    FAIL.set(false);
    embed_one(&s, 6);
    assert_eq!(BUILDS.get(), 2);
    assert!(matches!(*s.tier.read().unwrap(), Tier::Turbo(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Index build seam: search and save run on another thread mid-build; the vector saved
/// meanwhile must be in the swapped-in index.
#[test]
fn search_and_save_proceed_during_index_build() {
    use std::sync::{Arc, Mutex, mpsc};
    let dir = std::env::temp_dir().join(format!("memhub-mcp_tier_build_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = SqliteStore::open_with_tokenizer(&dir.join("m.db"), "porter unicode61").unwrap();
    let s = Arc::new(s.with_exact_max_chunks(6).unwrap());
    for i in 0..6 {
        embed_one(&s, i);
    }
    let result = Arc::new(Mutex::new(None));
    let (store, out) = (Arc::clone(&s), Arc::clone(&result));
    DURING.set(Some(Box::new(move || {
        let (other, (tx, rx)) = (Arc::clone(&store), mpsc::channel());
        std::thread::spawn(move || {
            let mut seen = 0;
            other.for_each_near(&SearchFilter::default(), &[0.1; 8], &mut |_, _, _| seen += 1).unwrap();
            tx.send((seen, embed_one(&other, 7))).unwrap();
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(10));
        *out.lock().unwrap() = Some(got.expect("search and save must not wait for the index build"));
    })));
    embed_one(&s, 6);
    DURING.set(None);
    let (seen, late) = result.lock().unwrap().take().expect("build seam ran");
    assert_eq!(seen, 7, "search during the build scans every stored vector");
    assert!(matches!(&*s.tier.read().unwrap(), Tier::Turbo(t) if t.len() == 8));
    let mut found = Vec::new();
    s.for_each_near(&SearchFilter::default(), &[0.1; 8], &mut |c, _, _| found.push(c)).unwrap();
    assert!(found.contains(&late));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A build cancelled by a reset must not swap in its stale index once a newer build started.
#[test]
fn cancelled_build_does_not_swap_over_newer_build() {
    use std::sync::Arc;
    let dir = std::env::temp_dir().join(format!("memhub-mcp_tier_stale_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = SqliteStore::open_with_tokenizer(&dir.join("m.db"), "porter unicode61").unwrap();
    let s = Arc::new(s.with_exact_max_chunks(6).unwrap());
    for i in 0..6 {
        embed_one(&s, i);
    }
    let other = Arc::clone(&s);
    DURING.set(Some(Box::new(move || {
        let other = Arc::clone(&other);
        std::thread::spawn(move || {
            other.reset_embeddings("other-model").unwrap();
            let starts: Vec<_> = (100..107).filter_map(|i| other.remember(ChunkId(i), &[0.1; 8]).unwrap()).collect();
            assert_eq!(starts.len(), 1, "the seventh vector starts a newer build");
        })
        .join()
        .unwrap();
    })));
    embed_one(&s, 6);
    DURING.set(None);
    assert!(matches!(*s.tier.read().unwrap(), Tier::Exact(7)), "stale index must not be swapped in");
    let _ = std::fs::remove_dir_all(&dir);
}
