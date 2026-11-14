//! Allowed FTS5 tokenizers.

use serde::{Deserialize, Serialize};

/// Allowed FTS5 tokenizers; anything else is rejected at load since it goes into SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FtsTokenizer {
    /// English stemming (default).
    #[serde(rename = "porter unicode61")]
    Porter,
    /// No stemming, accents folded: for non-English notes.
    #[serde(rename = "unicode61 remove_diacritics 2")]
    Unicode61,
}

impl FtsTokenizer {
    /// Gives the `tokenize=` argument spliced into the FTS5 table definition.
    ///
    /// # Returns
    /// A fixed, SQL-safe tokenizer spec matching the serde name of the variant.
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::Porter => "porter unicode61",
            Self::Unicode61 => "unicode61 remove_diacritics 2",
        }
    }
}

#[cfg(test)]
mod tests {
    use figment::Figment;
    use figment::providers::Serialized;

    use super::*;

    /// Proves only whitelisted tokenizer strings deserialize: a known spec maps to its variant and an SQL-injection string fails extraction.
    #[test]
    fn rejects_unknown_tokenizer() {
        let ok: std::result::Result<FtsTokenizer, _> =
            Figment::from(Serialized::default("t", "unicode61 remove_diacritics 2")).extract_inner("t");
        assert_eq!(ok.ok(), Some(FtsTokenizer::Unicode61));
        let bad: std::result::Result<FtsTokenizer, _> =
            Figment::from(Serialized::default("t", "trigram'); DROP TABLE notes;--")).extract_inner("t");
        assert!(bad.is_err());
    }
}
