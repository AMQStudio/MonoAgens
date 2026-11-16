//! Neutral `none` embedder returning no vector while keeping the cognitive loop alive.

use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};

use crate::embed::{EmbedError, EmbedInput, EmbedModality, Embedder, TaskPrefix};

const ALL_MODALITIES: [EmbedModality; 3] = [
    EmbedModality::Text,
    EmbedModality::Image,
    EmbedModality::AudioVoice,
];

/// No-op embedder (`neutral` embedding ablation): emits no vectors so vector-dependent
/// gate signals remain at `0.0` while structural signals keep the loop running.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoneEmbedder;

impl Embedder for NoneEmbedder {
    fn backend_name(&self) -> &str {
        "none"
    }

    fn model_ver(&self) -> Option<&str> {
        None
    }

    fn dim_d(&self) -> Option<u32> {
        None
    }

    fn max_context_tokens(&self) -> Option<usize> {
        None
    }

    fn supported_modalities(&self) -> &'static [EmbedModality] {
        &ALL_MODALITIES
    }

    fn tag_for(&self, _task: TaskPrefix) -> Option<EmbeddingTag> {
        None
    }

    fn embed(&self, _input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError> {
        Ok(None)
    }
}
