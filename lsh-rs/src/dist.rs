use crate::DataPoint;
use ndarray::prelude::*;
use rayon::prelude::*;

/// L2 norm of a single vector.
///
/// # Examples
///
/// ```
/// use lsh_rs::dist::l2_norm;
/// let a = vec![1., -1.];
/// let norm_a = l2_norm(&a);
///
/// // norm between two vectors
/// let b = vec![0.2, 1.2];
/// let c: Vec<f32> = a.iter().zip(b).map(|(ai, bi)| ai - bi).collect();
/// let norm_ab = l2_norm(&c);
/// ```
pub fn l2_norm(x: &[f32]) -> f32 {
    let x = aview1(x);
    x.dot(&x).sqrt()
}

/// Dot product between two vectors.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
///
/// # Examples
///
/// ```
/// use lsh_rs::dist::inner_prod;
/// let a = vec![1., -1.];
/// let b = vec![0.2, 1.2];
/// let prod = inner_prod(&a, &b);
/// ```
pub fn inner_prod(a: &[f32], b: &[f32]) -> f32 {
    aview1(a).dot(&aview1(b))
}

/// Cosine similarity between two vectors.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
///
/// # Examples
///
/// ```
/// use lsh_rs::dist::cosine_sim;
/// let a = vec![1., -1.];
/// let b = vec![0.2, 1.2];
/// let sim = cosine_sim(&a, &b);
/// ```
pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    inner_prod(a, b) / (l2_norm(a) * l2_norm(b))
}
