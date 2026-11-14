//! Splits a note body into overlapping token windows that fit the embedder.

use crate::domain::NewChunk;
use crate::error::Result;
use crate::ports::embedder::Tokenizer;

/// Tokens shared by consecutive chunks, at most an eighth of the window.
const OVERLAP: usize = 32;

/// Splits `body` into overlapping windows of at most [`Tokenizer::max_tokens`] tokens so each
/// chunk fits the embedder; consecutive windows share up to 32 tokens (never more than an
/// eighth of the window) so text near a window boundary keeps some context on both sides.
///
/// A window whose text, re-tokenised with the document prefix, still exceeds the embedder's
/// input ([`Tokenizer::document_overflow`] is nonzero) is shortened by that overflow until it
/// fits, so the embedder never truncates a stored chunk.
///
/// # Arguments
/// * `body` - Note text to split; chunk offsets are byte positions into it.
/// * `tokenizer` - Tokenizer of the embedder that will embed the chunks; supplies token byte
///   spans, the window size (treated as at least 1) and the overflow check.
///
/// # Returns
/// Chunks in order with `seq` 0, 1, 2…, each holding its byte range and text, covering every
/// token of `body`; empty when `body` has no tokens.
///
/// # Errors
/// * [`Error::Embedder`](crate::error::Error::Embedder) - The tokenizer fails on `body` or on a
///   candidate chunk.
pub fn split(body: &str, tokenizer: &dyn Tokenizer) -> Result<Vec<NewChunk>> {
    let spans = tokenizer.token_spans(body)?;
    let window = tokenizer.max_tokens().max(1);
    let stride = window - OVERLAP.min(window / 8);
    let mut out = Vec::new();
    let mut first = 0;
    while first < spans.len() {
        let mut last = (first + window).min(spans.len()) - 1;
        let mut text = body.get(spans[first].0..spans[last].1).unwrap_or_default();
        loop {
            let over = tokenizer.document_overflow(text)?;
            if over == 0 || last == first {
                break;
            }
            last = last.saturating_sub(over).max(first);
            text = body.get(spans[first].0..spans[last].1).unwrap_or_default();
        }
        let seq = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let (start, end) = (spans[first].0, spans[last].1);
        out.push(NewChunk { seq, start, end, text: text.to_owned() });
        if last + 1 == spans.len() {
            break;
        }
        first = (first + stride).min(last + 1);
    }
    Ok(out)
}
