//! The one place embeddings enter the app: each is L2-normalised here, whatever the adapter
//! returns, so stored and query embeddings are unit length and dot product equals cosine.

use crate::error::{Error, Result};
use crate::ports::embedder::{Embedder, Kind};

/// Embeds `text` with `embedder` and scales the vector to unit length, so every stored and
/// query vector can be compared by plain dot product. Runs model inference; the caller holds
/// the embedder lock.
///
/// # Arguments
/// * `embedder` - Model to run; borrowed mutably because inference may mutate its state.
/// * `text` - Chunk or query text to embed.
/// * `kind` - Whether `text` is a stored document chunk or a search query (models may prefix
///   them differently).
///
/// # Returns
/// The unit-length embedding.
///
/// # Errors
/// * [`Error::Embedder`] - Inference fails, or the vector is all zeros, empty, or holds a NaN
///   or infinite value.
pub fn embed(embedder: &mut dyn Embedder, text: &str, kind: Kind) -> Result<Vec<f32>> {
    let mut v = embedder.embed(text, kind)?;
    normalize(&mut v)?;
    Ok(v)
}

/// Scales `v` to unit L2 length in place, computing the norm in `f64` for precision.
///
/// # Arguments
/// * `v` - Vector to normalise; left untouched on error.
///
/// # Errors
/// * [`Error::Embedder`] - `v` is empty, all zeros, or its norm is not finite (a NaN or
///   infinite component).
pub fn normalize(v: &mut [f32]) -> Result<()> {
    let norm = v.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(Error::Embedder(format!("embedder returned an unusable vector (norm {norm})")));
    }
    v.iter_mut().for_each(|x| *x = (f64::from(*x) / norm) as f32);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves `normalize` turns the 3-4-5 vector `[3, 4]` into exactly `[0.6, 0.8]`.
    #[test]
    fn scales_to_unit_length() -> Result<()> {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v)?;
        assert_eq!(v, vec![0.6, 0.8]);
        Ok(())
    }

    /// Proves `normalize` errors on zero, empty, NaN-holding and infinity-holding inputs.
    #[test]
    fn rejects_zero_and_non_finite() {
        for mut v in [vec![0.0, 0.0], vec![], vec![1.0, f32::NAN], vec![f32::INFINITY, 0.0]] {
            assert!(normalize(&mut v).is_err(), "{v:?}");
        }
    }
}
