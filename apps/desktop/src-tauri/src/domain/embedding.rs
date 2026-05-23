//! `frame_embeddings` domain types (spec §五, Phase 4).
//!
//! The vector lives on disk as raw little-endian f32 bytes; in memory
//! we deal with `Vec<f32>`. `FrameEmbedding::from_bytes` /
//! `FrameEmbedding::to_bytes` are the only places that touch the LE
//! encoding so the rest of the codebase stays in float space.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct FrameEmbedding {
    pub frame_id: String,
    pub model: String,
    pub vector: Vec<f32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewFrameEmbedding {
    pub frame_id: String,
    pub model: String,
    pub vector: Vec<f32>,
}

/// Encode a vector as little-endian f32 bytes for the SQLite BLOB.
pub fn encode_vector(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vec.len() * 4);
    for v in vec {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a SQLite BLOB back into floats. Returns `None` if the byte
/// slice isn't a multiple of 4 (the row was written by a buggy producer
/// or a foreign tool — we'd rather skip it than silently truncate).
pub fn decode_vector(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().ok()?;
        out.push(f32::from_le_bytes(arr));
    }
    Some(out)
}

/// Cosine similarity between two same-length vectors. Returns `None`
/// if the lengths differ or either vector has zero norm.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return None;
    }
    Some(dot / denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_round_trips_through_blob() {
        let v = vec![0.1_f32, -0.5, 1.25, 3.14];
        let bytes = encode_vector(&v);
        let back = decode_vector(&bytes).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn decode_rejects_misaligned_bytes() {
        assert!(decode_vector(&[1, 2, 3]).is_none());
    }

    #[test]
    fn cosine_similarity_perfect_match_is_one() {
        let v = vec![1.0_f32, 2.0, 3.0];
        let s = cosine_similarity(&v, &v).unwrap();
        assert!((s - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        let s = cosine_similarity(&a, &b).unwrap();
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_mismatched_lengths_returns_none() {
        let a = vec![1.0_f32, 2.0, 3.0];
        let b = vec![1.0_f32, 2.0];
        assert!(cosine_similarity(&a, &b).is_none());
    }
}
