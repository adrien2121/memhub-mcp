//! Unit tests for [`super`].

use super::*;
use crate::app::fake::FakeStore;
use crate::domain::{NewNote, SearchFilter, Tag};
use crate::ports::store::NoteRecords;

/// Builds a keyword-only search service over a fake store holding three notes: id 1 `a`
/// ("rust sqlite", tag `x`), id 2 `b` ("rust only", tag `y`), id 3 `c` ("go", tag `x`).
///
/// # Returns
/// The service and the store, so tests can inspect recorded calls.
///
/// # Errors
/// * [`crate::error::Error::Storage`] - The fake store's mutex is poisoned.
fn service() -> Result<(SearchService, Arc<FakeStore>)> {
    let store = Arc::new(FakeStore::default());
    for (title, body, tag) in [("a", "rust sqlite", "x"), ("b", "rust only", "y"), ("c", "go", "x")] {
        let note = NewNote { title: title.into(), body: body.into(), tags: vec![Tag(tag.into())] };
        store.create(&note, &[], false)?;
    }
    Ok((SearchService::new(store.clone(), None), store))
}

/// Builds a search request.
///
/// # Arguments
/// * `query` - Free text.
/// * `limit` - Requested hit count, before clamping.
/// * `tag` - Optional tag filter.
///
/// # Returns
/// The request with no time bounds.
fn req(query: &str, limit: usize, tag: Option<&str>) -> SearchRequest {
    let filter = SearchFilter { tag: tag.map(|t| Tag(t.into())), ..Default::default() };
    SearchRequest { query: query.into(), limit, filter }
}

/// Proves keyword search finds both "rust" notes, flags the result keyword-only, and that a
/// tag filter keeps only the tagged note.
#[test]
fn keyword_search_with_tag_filter() -> Result<()> {
    let (svc, _) = service()?;
    let all = svc.search(&req("rust", 10, None))?;
    assert!(all.keyword_only);
    assert_eq!(all.hits.len(), 2);
    let tagged = svc.search(&req("rust", 10, Some("x")))?;
    assert_eq!(tagged.hits.iter().map(|h| h.note_id.0).collect::<Vec<_>>(), vec![1]);
    Ok(())
}

/// Proves a limit of 0 is raised to 1, a punctuation-only query returns nothing without
/// reaching the store, and the last store query was the quoted word.
#[test]
fn clamps_limit_and_skips_empty_query() -> Result<()> {
    let (svc, store) = service()?;
    assert_eq!(svc.search(&req("rust", 0, None))?.hits.len(), 1);
    assert!(svc.search(&req("?? ()", 10, None))?.hits.is_empty());
    let last = store.last_query.lock().map(|q| q.clone()).unwrap_or_default();
    assert_eq!(last.as_deref(), Some("\"rust\""));
    Ok(())
}

/// Proves `dropped_terms` counts the words past the term cap and is 0 for a short query.
#[test]
fn reports_words_past_the_term_cap() -> Result<()> {
    let (svc, _) = service()?;
    let long = format!("rust {}", vec!["w"; 40].join(" "));
    assert_eq!(svc.search(&req(&long, 10, None))?.dropped_terms, 41 - query::MAX_TERMS);
    assert_eq!(svc.search(&req("rust", 10, None))?.dropped_terms, 0);
    Ok(())
}

/// Proves snippets are fetched only for the hits returned after the limit (2 of 3 matches).
#[test]
fn snippets_only_for_returned_hits() -> Result<()> {
    let (svc, store) = service()?;
    let hits = svc.search(&req("rust go", 2, None))?.hits;
    assert_eq!(hits.len(), 2);
    assert_eq!(store.snippets.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(hits[0].snippet, "a");
    Ok(())
}

/// Proves query text past the byte cap is ignored, with the cut backed off to a character
/// boundary inside a two-byte character, so a word after the cap finds nothing.
#[test]
fn ignores_query_past_the_byte_cap() -> Result<()> {
    let (svc, _) = service()?;
    // The 2 KiB cap falls inside the last "é": the cut must back off to a char boundary.
    let long = format!("a{} rust", "é".repeat(crate::domain::MAX_QUERY / 2));
    assert_eq!(crate::domain::truncate_query(&long).len(), crate::domain::MAX_QUERY - 1);
    assert!(svc.search(&req(&long, 10, None))?.hits.is_empty());
    Ok(())
}
