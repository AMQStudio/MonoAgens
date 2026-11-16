//! Deterministic fixture-driven `scripted` embedder for offline tests and Lab scenarios.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};

use crate::embed::{
    EmbedContent, EmbedError, EmbedInput, EmbedModality, Embedder, TaskPrefix, l2_normalize,
};

const ALL_MODALITIES: [EmbedModality; 3] = [
    EmbedModality::Text,
    EmbedModality::Image,
    EmbedModality::AudioVoice,
];

/// Deterministic offline fixture embedder supporting keyed vectors, queued responses,
/// and injected per-event failures.
#[derive(Debug)]
pub struct ScriptedEmbedder {
    model_ver: String,
    dim_d: u32,
    normalize: bool,
    by_text: BTreeMap<(TaskPrefix, String), Result<Vec<f32>, EmbedError>>,
    queue: Mutex<VecDeque<Result<Vec<f32>, EmbedError>>>,
    fallback: Option<Result<Vec<f32>, EmbedError>>,
}

impl ScriptedEmbedder {
    /// Creates a scripted embedder tagged with `(model_ver, dim_d, task)`.
    #[must_use]
    pub fn new(model_ver: impl Into<String>, dim_d: u32) -> Self {
        Self {
            model_ver: model_ver.into(),
            dim_d: dim_d.max(1),
            normalize: true,
            by_text: BTreeMap::new(),
            queue: Mutex::new(VecDeque::new()),
            fallback: None,
        }
    }

    /// Configures whether returned vectors are L2-normalized (default `true`).
    #[must_use]
    pub const fn with_normalize(mut self, normalize: bool) -> Self {
        self.normalize = normalize;
        self
    }

    /// Registers a deterministic vector for `(task, text)`.
    pub fn insert_text(&mut self, task: TaskPrefix, text: impl Into<String>, values: Vec<f32>) {
        self.by_text.insert((task, text.into()), Ok(values));
    }

    /// Registers an injected error for `(task, text)`.
    pub fn insert_error(&mut self, task: TaskPrefix, text: impl Into<String>, err: EmbedError) {
        self.by_text.insert((task, text.into()), Err(err));
    }

    /// Enqueues a sequential vector or failure response.
    pub fn push_response(&self, resp: Result<Vec<f32>, EmbedError>) {
        self.queue
            .lock()
            .expect("scripted embedder mutex")
            .push_back(resp);
    }

    /// Sets a fallback vector or error when no keyed entry or queued item matches.
    pub fn set_fallback(&mut self, resp: Result<Vec<f32>, EmbedError>) {
        self.fallback = Some(resp);
    }

    fn finalize_vec(
        &self,
        task: TaskPrefix,
        mut values: Vec<f32>,
    ) -> Result<TaggedEmbedding, EmbedError> {
        if values.len() != self.dim_d as usize {
            return Err(EmbedError::DimensionMismatch {
                expected: self.dim_d,
                actual: values.len(),
            });
        }
        if self.normalize {
            l2_normalize(&mut values)?;
        }
        Ok(TaggedEmbedding {
            tag: self.tag_for(task).expect("tag present"),
            values,
        })
    }
}

impl Embedder for ScriptedEmbedder {
    fn backend_name(&self) -> &str {
        "scripted"
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
        if let EmbedContent::Text(text) = input.content
            && let Some(entry) = self.by_text.get(&(input.task, text.to_owned()))
        {
            let values = entry.clone()?;
            return self.finalize_vec(input.task, values).map(Some);
        }

        if let Some(next) = self
            .queue
            .lock()
            .expect("scripted embedder mutex")
            .pop_front()
        {
            let values = next?;
            return self.finalize_vec(input.task, values).map(Some);
        }

        if let Some(fb) = &self.fallback {
            let values = fb.clone()?;
            return self.finalize_vec(input.task, values).map(Some);
        }

        Err(EmbedError::ScriptedFailure {
            detail: "no scripted embedding fixture registered for input".to_owned(),
        })
    }
}
