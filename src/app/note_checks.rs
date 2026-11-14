//! Input validation shared by the note service's write and edit paths.

use crate::domain::MAX_BODY;
use crate::error::{Error, Result};

/// Longest accepted title, in characters.
const MAX_TITLE: usize = 200;

/// Rejects a title that is blank or longer than 200 characters (Unicode scalar values).
///
/// # Arguments
/// * `title` - Proposed note title.
///
/// # Errors
/// * [`Error::InvalidInput`] - The title is empty or whitespace only, or too long.
pub(crate) fn check_title(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(Error::InvalidInput("title is empty".into()));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(Error::InvalidInput(format!("title longer than {MAX_TITLE} characters")));
    }
    Ok(())
}

/// Rejects a body larger than [`MAX_BODY`] bytes of UTF-8.
///
/// # Arguments
/// * `body` - Proposed note body, after any edit is applied.
///
/// # Errors
/// * [`Error::TooLarge`] - The body exceeds the limit; carries its byte length.
pub(crate) fn check_body(body: &str) -> Result<()> {
    if body.len() > MAX_BODY {
        return Err(Error::TooLarge(body.len()));
    }
    Ok(())
}
