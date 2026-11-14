//! Turns model output into one text vector: pooling and dimension check; the app normalises.

use ort::value::DynValue;

use super::err;
use crate::config::{EmbedderConfig, Pooling};
use crate::error::Result;

/// Pools `[1, len, dim]` hidden states into one vector, or passes a `[1, dim]` output through.
///
/// CLS pooling takes the first token's vector; mean pooling averages all token vectors.
///
/// # Arguments
/// * `cfg` - Supplies `dim` and `pooling`.
/// * `out` - First model output tensor, `f32`.
///
/// # Returns
/// An unnormalised vector of exactly `dim` floats.
///
/// # Errors
/// * [`crate::error::Error::Embedder`] - The output is not an `f32` tensor, has a rank other
///   than 2 or 3, or yields a length different from `dim`.
pub(super) fn pool(cfg: &EmbedderConfig, out: &DynValue) -> Result<Vec<f32>> {
    let (shape, data) = out.try_extract_tensor::<f32>().map_err(err)?;
    let dim = cfg.dim;
    let v = match (shape.len(), cfg.pooling) {
        (2, _) => data.to_vec(),
        (3, Pooling::Cls) => data.get(..dim).map(<[f32]>::to_vec).unwrap_or_default(),
        (3, Pooling::Mean) => {
            let rows = data.len() / dim.max(1);
            let mut sum = vec![0f32; dim];
            for row in data.chunks_exact(dim) {
                sum.iter_mut().zip(row).for_each(|(s, x)| *s += x);
            }
            sum.iter().map(|s| s / rows.max(1) as f32).collect()
        }
        _ => return Err(err(format!("unexpected output shape {shape:?}"))),
    };
    if v.len() != dim {
        return Err(err(format!("output dim {} != configured dim {dim}", v.len())));
    }
    Ok(v)
}
