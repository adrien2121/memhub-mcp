//! SHA-256 verification of setup downloads against pinned hashes.

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Verifies that `bytes` hash to the pinned SHA-256.
///
/// # Arguments
/// * `what` - Label for the error message (a URL or destination path).
/// * `bytes` - Content to verify.
/// * `expected` - Pinned lowercase hex SHA-256.
///
/// # Returns
/// `Ok(())` when the hashes match.
///
/// # Errors
/// * [`Error::InvalidInput`] - The hashes differ; the message shows both.
pub(super) fn check(what: &str, bytes: &[u8], expected: &str) -> Result<()> {
    let actual = sha256(bytes);
    let msg = || format!("sha256 mismatch for {what}: expected {expected}, got {actual}");
    if actual == expected { Ok(()) } else { Err(Error::InvalidInput(msg())) }
}

/// Hashes `bytes` with SHA-256.
///
/// # Arguments
/// * `bytes` - Content to hash.
///
/// # Returns
/// The digest as 64 lowercase hex characters.
pub(super) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
