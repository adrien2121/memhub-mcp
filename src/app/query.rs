//! Builds safe FTS5 `OR` queries from free text.

/// Most terms kept in one query.
pub const MAX_TERMS: usize = 32;

/// An FTS5 query and how many words past [`MAX_TERMS`] it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fts {
    /// FTS5 MATCH expression.
    pub query: String,
    /// Words dropped by the cap.
    pub dropped: usize,
}

/// Builds an FTS5 query matching any of the first [`MAX_TERMS`] words of `text`. Words are runs
/// of Unicode alphanumeric characters; each is double-quoted (inner quotes doubled) and joined
/// with `OR`, so FTS5 operators and punctuation in user text are always matched as plain text.
///
/// # Arguments
/// * `text` - Free-text search query from the user.
///
/// # Returns
/// The query and the count of words past the cap, or `None` when `text` holds no word,
/// meaning keyword search should be skipped.
pub fn build(text: &str) -> Option<Fts> {
    let mut words = text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty());
    let terms: Vec<String> =
        words.by_ref().take(MAX_TERMS).map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect();
    let dropped = words.count();
    (!terms.is_empty()).then(|| Fts { query: terms.join(" OR "), dropped })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs [`build`] and keeps only the query string.
    ///
    /// # Arguments
    /// * `text` - Free text to turn into a query.
    ///
    /// # Returns
    /// The FTS5 expression, or `None` when `text` holds no word.
    fn query(text: &str) -> Option<String> {
        build(text).map(|f| f.query)
    }

    /// Proves each word is quoted and words are joined with `OR`, including the operator word `AND`.
    #[test]
    fn quotes_and_joins_words() {
        assert_eq!(query("rust AND sqlite").as_deref(), Some("\"rust\" OR \"AND\" OR \"sqlite\""));
    }

    /// Proves empty, blank, punctuation-only and emoji-only inputs yield `None`.
    #[test]
    fn punctuation_only_is_none() {
        for input in ["", "   ", "? - / , \" : ( ) * ^", "🦀🎉"] {
            assert_eq!(build(input), None, "{input:?}");
        }
    }

    /// Proves FTS5 syntax (`NEAR(...)`, `NOT`, `*`) is reduced to quoted plain words.
    #[test]
    fn operators_are_quoted() {
        let q = query("NEAR(a b) NOT c*").unwrap_or_default();
        assert_eq!(q, "\"NEAR\" OR \"a\" OR \"b\" OR \"NOT\" OR \"c\"");
    }

    /// Proves a 100-word input keeps exactly [`MAX_TERMS`] terms and reports the rest as dropped,
    /// and that a short input reports zero dropped.
    #[test]
    fn caps_terms_and_counts_dropped() {
        let fts = build(&vec!["w"; 100].join(" ")).unwrap_or_else(|| Fts { query: String::new(), dropped: 0 });
        assert_eq!(fts.query.matches(" OR ").count(), MAX_TERMS - 1);
        assert_eq!(fts.dropped, 100 - MAX_TERMS);
        assert_eq!(build("a b").map(|f| f.dropped), Some(0));
    }

    /// Proves accented letters stay inside words while an em dash separates them.
    #[test]
    fn keeps_unicode_words() {
        assert_eq!(query("café—naïve").as_deref(), Some("\"café\" OR \"naïve\""));
    }
}
