//! `setup`: downloads the default embedding model and onnxruntime, pinned by SHA-256, then
//! connects memhub-mcp to the user's AI tools (menu on a terminal, defaults otherwise).

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::config::{Config, EmbedderConfig, data_dir};
use crate::error::{Error, Result};

mod assets;
mod connect;
mod fetch;
mod menu;
#[cfg(test)]
mod menu_tests;
mod register;
#[cfg(test)]
mod tests;
mod verify;

use assets::{Asset, MODEL, ORT, TOKENIZER};
use fetch::{download, extract};
use verify::{check, sha256};

/// Largest accepted download, in bytes (256 MiB).
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

/// Runs `memhub-mcp setup`: installs the embedding model (`model.onnx`), its `tokenizer.json`
/// and the onnxruntime library for this platform at their default paths, skipping files already
/// present with the pinned hash. A configured `model_dir` or `ort_dylib` that differs from the
/// default is user-managed and left untouched. Then connects this binary (absolute path) to the
/// AI tools and prints one line per tool. When stdin and stdout are terminals and `yes` is
/// unset, a menu picks the tools and whether to add the usage rule; otherwise every found tool
/// is connected and the rule is added only with `rules` or `yes`. Output goes to stderr.
///
/// # Arguments
/// * `config` - Supplies `embedder.model_dir` (model and tokenizer) and `embedder.ort_dylib`
///   (onnxruntime library path).
/// * `paths` - `--path <client>=<path>` overrides, validated before any download.
/// * `rules` - `--rules`: without the menu, also add the usage rule (the menu asks instead).
/// * `yes` - `--yes`: skip the menu and accept its defaults, rule included.
///
/// # Returns
/// `Ok(())` when every default-path file is installed and verified.
///
/// # Errors
/// * [`Error::InvalidInput`] - Invalid `--path`, home directory unknown, no pinned onnxruntime 1.31 build for
///   this OS/arch, a hash mismatch, an archive member is missing, or a destination has no parent.
/// * [`Error::Io`] - A download, archive read, directory creation or file write failed.
pub fn run(config: &Config, paths: &[String], rules: bool, yes: bool) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| Error::InvalidInput("home directory is unknown".into()))?;
    let paths = register::overrides::parse(paths, &home)?;
    install_assets(config)?;
    let app_config = dirs::config_dir().ok_or_else(|| Error::InvalidInput("config directory is unknown".into()))?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from);
    let env = connect::Env { home: &home, app_config: &app_config, codex_home, exe: &exe };
    let ask = !yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mode = if ask { connect::Mode::Ask } else { connect::Mode::Defaults { rules: rules || yes } };
    let (stdin, mut stderr) = (std::io::stdin(), std::io::stderr());
    connect::connect(&env, paths, mode, &mut stdin.lock(), &mut stderr, &mut register::run_command)?;
    Ok(())
}

/// Installs the model, tokenizer and onnxruntime at their default paths (see [`run`]).
///
/// # Arguments
/// * `config` - Supplies the embedder paths.
///
/// # Returns
/// `Ok(())` when every default-path file is installed and verified.
///
/// # Errors
/// Same as [`run`].
fn install_assets(config: &Config) -> Result<()> {
    let defaults = EmbedderConfig::defaults(&data_dir()?);
    let cfg = &config.embedder;
    if cfg.model_dir == defaults.model_dir {
        for (asset, file) in [(&MODEL, "model.onnx"), (&TOKENIZER, "tokenizer.json")] {
            install(asset, &cfg.model_dir.join(file))?;
        }
    } else {
        eprintln!("custom model_dir, left untouched: {}", cfg.model_dir.display());
    }
    if cfg.ort_dylib != defaults.ort_dylib {
        eprintln!("custom ort_dylib, left untouched: {}", cfg.ort_dylib.display());
        return Ok(());
    }
    let ort = ORT.ok_or_else(|| {
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        Error::InvalidInput(format!(
            "no onnxruntime 1.31 build for {os}-{arch}; supported: macOS arm64, Linux x64/arm64, Windows x64"
        ))
    })?;
    install(&ort, &cfg.ort_dylib)
}

/// Installs one pinned asset at `dest`: skips it if `dest` already has the pinned hash;
/// otherwise downloads it, verifies the archive hash and extracts the member when the asset is
/// an archive, verifies the file hash, then writes `dest.part` and renames it over `dest` so a
/// failed write never leaves a partial file at `dest`.
///
/// # Arguments
/// * `asset` - URL, optional archive hash/member, and the installed file's hash.
/// * `dest` - Final path; parent directories are created.
///
/// # Returns
/// `Ok(())` when `dest` holds the verified file.
///
/// # Errors
/// * [`Error::InvalidInput`] - Hash mismatch, missing archive member, or `dest` has no parent.
/// * [`Error::Io`] - Download, extraction, directory creation, write or rename failed.
fn install(asset: &Asset, dest: &Path) -> Result<()> {
    if fs::read(dest).is_ok_and(|bytes| sha256(&bytes) == asset.sha256) {
        eprintln!("present: {}", dest.display());
        return Ok(());
    }
    eprintln!("downloading {}", asset.url);
    let download = download(asset.url)?;
    let bytes = match asset.archive {
        None => download,
        Some((archive_sha256, member)) => {
            check(asset.url, &download, archive_sha256)?;
            extract(asset.url, &download, member)?
        }
    };
    check(dest.to_string_lossy().as_ref(), &bytes, asset.sha256)?;
    let dir = dest.parent().ok_or_else(|| Error::InvalidInput(format!("no parent: {}", dest.display())))?;
    fs::create_dir_all(dir)?;
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".part");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, dest)?;
    eprintln!("installed: {}", dest.display());
    Ok(())
}
