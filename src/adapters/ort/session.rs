//! onnxruntime session with the low-memory settings measured on bge-small int8.

use std::path::Path;

use ort::ep::CPU;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;

use super::err;
use crate::error::Result;

/// Loads `dylib`, then `onnx` with graph optimisation and memory pattern off, CPU arena on.
///
/// Over 84 510-token documents the arena peaks ~14 MB below plain malloc, with bit-identical
/// vectors. Optimisation levels above `Disable` peak near 1 GB on the first document.
///
/// # Arguments
/// * `dylib` - Path to the onnxruntime shared library.
/// * `onnx` - Path to `model.onnx`.
/// * `threads` - Intra-op thread count.
///
/// # Returns
/// A session ready for inference.
///
/// # Errors
/// * [`crate::error::Error::Embedder`] - The library or model cannot be loaded, or a session
///   option is rejected.
pub(super) fn open(dylib: &Path, onnx: &Path, threads: usize) -> Result<Session> {
    let env = ort::init_from(dylib).map_err(err)?.build().map_err(err)?;
    let cpu = CPU::default().with_arena_allocator(true).build();
    Session::builder(&env)
        .map_err(err)?
        .with_optimization_level(GraphOptimizationLevel::Disable)
        .map_err(err)?
        .with_memory_pattern(false)
        .map_err(err)?
        .with_intra_threads(threads)
        .map_err(err)?
        .with_execution_providers([cpu])
        .map_err(err)?
        .commit_from_file(onnx)
        .map_err(err)
}
