//! Fetching setup assets: HTTPS download and archive member extraction.

use std::io::{self, Read};
use std::path::Path;

use super::MAX_DOWNLOAD;
use crate::error::{Error, Result};

/// Downloads `url` fully into memory over HTTPS, refusing bodies over [`MAX_DOWNLOAD`] bytes.
///
/// # Arguments
/// * `url` - Pinned asset or archive URL.
///
/// # Returns
/// The response body bytes.
///
/// # Errors
/// * [`Error::Io`] - The request failed, returned an error status, or the body exceeded the
///   limit; the message is prefixed with `url`.
pub(super) fn download(url: &str) -> Result<Vec<u8>> {
    ureq::get(url)
        .call()
        .and_then(|mut r| r.body_mut().with_config().limit(MAX_DOWNLOAD).read_to_vec())
        .map_err(|e| io::Error::other(format!("{url}: {e}")).into())
}

/// Pulls one member file out of a downloaded archive held in memory: a gzipped tar when `url`
/// ends in `.tgz`, otherwise a zip (also used for `.whl` wheels).
///
/// # Arguments
/// * `url` - The archive's URL; selects the format and labels errors.
/// * `archive` - The archive bytes.
/// * `member` - Exact path of the wanted file inside the archive.
///
/// # Returns
/// The member's uncompressed bytes.
///
/// # Errors
/// * [`Error::Io`] - The archive is corrupt or unreadable.
/// * [`Error::InvalidInput`] - No entry named `member` exists.
pub(super) fn extract(url: &str, archive: &[u8], member: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if url.ends_with(".tgz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in tar.entries()? {
            let mut entry = entry?;
            if entry.path()?.as_ref() == Path::new(member) {
                entry.read_to_end(&mut out)?;
                return Ok(out);
            }
        }
    } else {
        let mut zip = zip::ZipArchive::new(io::Cursor::new(archive)).map_err(io::Error::other)?;
        if let Ok(mut file) = zip.by_name(member) {
            file.read_to_end(&mut out)?;
            return Ok(out);
        }
    }
    Err(Error::InvalidInput(format!("{member} not found in {url}")))
}
