//! Deterministic offline `hash` embedder using keyed BLAKE3 projection.

use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};

use crate::embed::{
    EmbedContent, EmbedError, EmbedInput, EmbedModality, Embedder, TaskPrefix, l2_normalize,
};

const ALL_MODALITIES: [EmbedModality; 3] = [
    EmbedModality::Text,
    EmbedModality::Image,
    EmbedModality::AudioVoice,
];

/// Deterministic offline embedder projecting text/media into L2-normalized `dim_d` vectors via BLAKE3.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    model_ver: String,
    dim_d: u32,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self::new(256)
    }
}

impl HashEmbedder {
    /// Creates a deterministic `HashEmbedder` with the given output dimension (default `256`).
    #[must_use]
    pub fn new(dim_d: u32) -> Self {
        Self {
            model_ver: "hash-blake3-v1".to_owned(),
            dim_d: dim_d.max(1),
        }
    }

    /// Creates a `HashEmbedder` with an explicit `embed_model_ver` and dimension.
    #[must_use]
    pub fn with_model_ver(model_ver: impl Into<String>, dim_d: u32) -> Self {
        Self {
            model_ver: model_ver.into(),
            dim_d: dim_d.max(1),
        }
    }
}

impl Embedder for HashEmbedder {
    fn backend_name(&self) -> &str {
        "hash"
    }

    fn model_ver(&self) -> Option<&str> {
        Some(&self.model_ver)
    }

    fn dim_d(&self) -> Option<u32> {
        Some(self.dim_d)
    }

    fn max_context_tokens(&self) -> Option<usize> {
        None
    }

    fn supported_modalities(&self) -> &'static [EmbedModality] {
        &ALL_MODALITIES
    }

    fn tag_for(&self, task: TaskPrefix) -> Option<EmbeddingTag> {
        Some(EmbeddingTag {
            embed_model_ver: self.model_ver.clone(),
            dim_d: self.dim_d,
            prompt_regime_id: task.regime_id().to_owned(),
        })
    }

    fn embed(&self, input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"monoagens.hash_embed.v1");
        hasher.update(self.model_ver.as_bytes());
        hasher.update(&self.dim_d.to_le_bytes());
        hasher.update(input.task.regime_id().as_bytes());

        match input.content {
            EmbedContent::Text(text) => {
                hasher.update(b"text");
                let prefixed = input.task.format_text(text);
                hasher.update(prefixed.as_bytes());
            }
            EmbedContent::Image {
                bytes,
                mime_type,
                caption,
            } => {
                hasher.update(b"image");
                hasher.update(mime_type.as_bytes());
                hasher.update(bytes);
                if let Some(cap) = caption {
                    hasher.update(input.task.format_text(cap).as_bytes());
                }
            }
            EmbedContent::AudioVoice {
                bytes,
                mime_type,
                transcript,
            } => {
                hasher.update(b"audio_voice");
                hasher.update(mime_type.as_bytes());
                hasher.update(bytes);
                if let Some(tr) = transcript {
                    hasher.update(input.task.format_text(tr).as_bytes());
                }
            }
        }

        let mut xof = hasher.finalize_xof();
        let dim = self.dim_d as usize;
        let mut buf = vec![0_u8; dim * 4];
        xof.fill(&mut buf);

        let mut values = Vec::with_capacity(dim);
        let (chunks, _) = buf.as_chunks::<4>();
        for chunk in chunks {
            let raw = i32::from_le_bytes(*chunk);
            // Map symmetrically into [-1.0, 1.0].
            values.push((f64::from(raw) / f64::from(i32::MAX)) as f32);
        }
        l2_normalize(&mut values)?;

        Ok(Some(TaggedEmbedding {
            tag: self.tag_for(input.task).expect("tag present"),
            values,
        }))
    }
}
