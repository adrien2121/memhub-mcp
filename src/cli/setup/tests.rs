//! Tests for setup's hash checking and its skip-when-present install path.

use super::{Asset, check, install, install_assets, run};
use crate::config::{Config, data_dir};
use crate::error::{Error, Result};

/// SHA-256 of the bytes `abc`.
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// Proves a hash mismatch is reported as [`Error::InvalidInput`]: hashing `abd` against the
/// hash of `abc` fails.
#[test]
fn check_rejects_mismatched_hash() {
    let res = check("f", b"abd", ABC);
    assert!(matches!(res, Err(Error::InvalidInput(_))), "{res:?}");
}

/// Proves `check` accepts bytes whose SHA-256 equals the pinned hash (`abc` against [`ABC`]).
#[test]
fn check_accepts_matching_hash() -> Result<()> {
    check("f", b"abc", ABC)
}

/// Proves `install` leaves an existing file with the pinned hash alone without downloading:
/// the asset has an empty URL, so any download attempt would fail, yet `install` returns `Ok`.
#[test]
fn install_skips_download_when_present_file_matches() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_setup_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join("asset");
    // Empty URL: a download attempt would error, so Ok proves the present file was accepted.
    let asset = Asset { url: "", archive: None, sha256: ABC };
    std::fs::write(&dest, b"abc")?;
    let res = install(&asset, &dest);
    std::fs::remove_dir_all(&dir)?;
    res
}

/// Proves setup leaves user-configured custom paths alone: with `model_dir` and `ort_dylib`
/// pointing outside the real defaults and holding non-pinned bytes, `install_assets` returns `Ok` without
/// downloading, and both config values and file bytes are unchanged.
#[test]
fn run_preserves_custom_model_and_ort_paths() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_setup_custom_{}", std::process::id()));
    let custom = dir.join("custom");
    std::fs::create_dir_all(&custom)?;
    let mut config = Config::defaults(&data_dir()?);
    config.embedder.model_dir = custom.join("model");
    config.embedder.ort_dylib = custom.join("libort.custom");
    std::fs::create_dir_all(&config.embedder.model_dir)?;
    let files = [
        (config.embedder.model_dir.join("model.onnx"), b"my model".as_slice()),
        (config.embedder.model_dir.join("tokenizer.json"), b"my tokenizer".as_slice()),
        (config.embedder.ort_dylib.clone(), b"my ort".as_slice()),
    ];
    for (path, bytes) in &files {
        std::fs::write(path, bytes)?;
    }
    let (model_dir, ort_dylib) = (config.embedder.model_dir.clone(), config.embedder.ort_dylib.clone());
    let res = install_assets(&config);
    let kept: Vec<Vec<u8>> = files.iter().map(|(p, _)| std::fs::read(p)).collect::<std::io::Result<_>>()?;
    std::fs::remove_dir_all(&dir)?;
    res?;
    assert_eq!((config.embedder.model_dir, config.embedder.ort_dylib), (model_dir, ort_dylib));
    for ((_, bytes), got) in files.iter().zip(kept) {
        assert_eq!(got, *bytes);
    }
    Ok(())
}

/// Proves an unknown `--path` client fails before any download: the model dir points at an
/// empty temp dir, so an install would download into it, yet it stays empty.
#[test]
fn run_rejects_unknown_client_before_download() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("memhub-mcp_setup_bogus_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let mut config = Config::defaults(&data_dir()?);
    config.embedder.model_dir = dir.clone();
    let res = run(&config, &["bogus=x".into()], false, false);
    let empty = std::fs::read_dir(&dir)?.next().is_none();
    std::fs::remove_dir_all(&dir)?;
    assert!(matches!(&res, Err(Error::InvalidInput(m)) if m.contains("vscode")), "{res:?}");
    assert!(empty);
    Ok(())
}
