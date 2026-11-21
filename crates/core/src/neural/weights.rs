//! Dimension-agnostic `safetensors` weight loader with `BLAKE3` verification,
//! shape validation, and finite-weight enforcement.

use std::collections::HashMap;

use safetensors::{Dtype, SafeTensors};
use serde::{Deserialize, Serialize};

use crate::blake3;
use crate::neural::NeuralError;

/// Supported per-place recurrent working-memory cell architectures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellType {
    /// Minimal GRU (`arXiv:2410.01201`).
    MinGru,
    /// Decoupled channel-wise erase/write `Gated DeltaNet-2` (`arXiv:2605.22791`).
    GatedDeltaNet2,
}

impl CellType {
    /// Parses the canonical metadata `cell_type` string.
    pub fn from_metadata_str(s: &str) -> Result<Self, NeuralError> {
        match s {
            "mingru" => Ok(Self::MinGru),
            "gated_deltanet2" => Ok(Self::GatedDeltaNet2),
            other => Err(NeuralError::InvalidMetadata {
                key: "cell_type",
                detail: format!("unsupported cell_type '{other}', expected 'mingru' or 'gated_deltanet2'"),
            }),
        }
    }

    /// Canonical `safetensors` metadata string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MinGru => "mingru",
            Self::GatedDeltaNet2 => "gated_deltanet2",
        }
    }
}

/// Validated architecture & calibration metadata parsed from `safetensors` `__metadata__`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeuralModelConfig {
    pub feature_schema_version: u16,
    pub d_in_emb: usize,
    pub d_in_num: usize,
    pub plr_n_frequencies: usize,
    pub d_e: usize,
    pub d_n: usize,
    pub d_model: usize,
    pub d_ff: usize,
    pub n_layers: usize,
    pub tabm_k: usize,
    pub h_dim: usize,
    #[serde(default)]
    pub h_actor_dim: usize,
    pub cell_type: CellType,
    pub temperature: f32,
    pub cic_threshold: f32,
    pub max_wait_ms: f32,
    pub has_phase2_heads: bool,
    pub has_phase3_heads: bool,
    #[serde(default)]
    pub has_v22_heads: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_embed_model_ver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_prompt_regime_id: Option<String>,
}

/// One Pre-LN Residual SwiGLU block with `TabM` rank-1 ensemble adapters.
#[derive(Debug, Clone, PartialEq)]
pub struct SwiGluBlockWeights {
    pub norm_weight: Vec<f32>,
    pub norm_bias: Vec<f32>,
    pub w_gate: Vec<f32>,
    pub w_up: Vec<f32>,
    pub w_down: Vec<f32>,
    pub tabm_r: Vec<f32>,
    pub tabm_s: Vec<f32>,
}

/// Loaded recurrent cell weights for either `minGRU` or `Gated DeltaNet-2`.
#[derive(Debug, Clone, PartialEq)]
pub enum RecurrentCellWeights {
    MinGru {
        w_z_weight: Vec<f32>,
        w_z_bias: Vec<f32>,
        w_h_weight: Vec<f32>,
        w_h_bias: Vec<f32>,
    },
    GatedDeltaNet2 {
        w_erase_weight: Vec<f32>,
        w_erase_bias: Vec<f32>,
        w_write_weight: Vec<f32>,
        w_write_bias: Vec<f32>,
        w_delta_weight: Vec<f32>,
        w_delta_bias: Vec<f32>,
    },
}

/// Optional Phase 2 metacognitive heads (`p_uncertainty`, `p_verify`).
#[derive(Debug, Clone, PartialEq)]
pub struct Phase2HeadWeights {
    pub p_uncertainty_weight: Vec<f32>,
    pub p_uncertainty_bias: f32,
    pub p_verify_weight: Vec<f32>,
    pub p_verify_bias: f32,
}

/// Optional Phase 3 memory gating heads (`p_encode_importance`, `p_memory_relevance`).
#[derive(Debug, Clone, PartialEq)]
pub struct Phase3HeadWeights {
    pub p_encode_importance_weight: Vec<f32>,
    pub p_encode_importance_bias: f32,
    pub p_memory_relevance_weight: Vec<f32>,
    pub p_memory_relevance_bias: f32,
}

/// Optional V2.2 cognitive control heads (`compute_tier`, `p_user_correction`).
#[derive(Debug, Clone, PartialEq)]
pub struct V22HeadWeights {
    pub compute_tier_weight: Vec<f32>,
    pub compute_tier_bias: [f32; 3],
    pub p_user_correction_weight: Vec<f32>,
    pub p_user_correction_bias: f32,
}

/// Fully validated, immutable Pure-Rust neural controller weights loaded from `safetensors`.
#[derive(Debug, Clone, PartialEq)]
pub struct NeuralModel {
    pub config: NeuralModelConfig,
    pub blake3: [u8; 32],
    // Stem
    pub emb_linear_weight: Vec<f32>,
    pub emb_linear_bias: Vec<f32>,
    pub emb_norm_weight: Vec<f32>,
    pub emb_norm_bias: Vec<f32>,
    pub plr_freqs: Vec<f32>,
    pub plr_linear_weight: Vec<f32>,
    pub plr_linear_bias: Vec<f32>,
    pub fusion_weight: Vec<f32>,
    pub fusion_bias: Vec<f32>,
    // Trunk
    pub tabm_in_scale: Vec<f32>,
    pub blocks: Vec<SwiGluBlockWeights>,
    pub final_norm_weight: Vec<f32>,
    pub final_norm_bias: Vec<f32>,
    // Per-place working memory cell
    pub memory_cell: RecurrentCellWeights,
    // Optional per-actor working memory cell (when h_actor_dim > 0)
    pub actor_memory_cell: Option<RecurrentCellWeights>,
    // Phase 1 heads (required)
    pub gate_4_weight: Vec<f32>,
    pub gate_4_bias: [f32; 4],
    pub wait_delta_t_weight: Vec<f32>,
    pub wait_delta_t_bias: f32,
    // Optional Phase 2, Phase 3 & V2.2 heads
    pub phase2: Option<Phase2HeadWeights>,
    pub phase3: Option<Phase3HeadWeights>,
    pub v22_heads: Option<V22HeadWeights>,
}

impl NeuralModel {
    /// Deserializes and validates a `safetensors` byte buffer, verifying `BLAKE3` if `expected_blake3`
    /// is provided, enforcing `F32` dtype, exact tensor shapes, and all-finite weights & scalars.
    pub fn from_safetensors_bytes(
        bytes: &[u8],
        expected_blake3: Option<[u8; 32]>,
    ) -> Result<Self, NeuralError> {
        let actual_blake3 = blake3::hash(bytes);
        if let Some(expected) = expected_blake3
            && actual_blake3 != expected
        {
            return Err(NeuralError::Blake3Mismatch {
                expected: blake3::to_hex(&expected),
                actual: blake3::to_hex(&actual_blake3),
            });
        }

        let (_header_len, raw_metadata) =
            SafeTensors::read_metadata(bytes).map_err(|e| NeuralError::SafeTensorParse(e.to_string()))?;
        let meta_map = raw_metadata
            .metadata()
            .as_ref()
            .ok_or(NeuralError::MissingMetadata("__metadata__"))?;

        let tensors =
            SafeTensors::deserialize(bytes).map_err(|e| NeuralError::SafeTensorParse(e.to_string()))?;

        let feature_schema_version = parse_u16_gt_zero(meta_map, "feature_schema_version")?;
        if feature_schema_version != 1 && feature_schema_version != 2 {
            return Err(NeuralError::InvalidMetadata {
                key: "feature_schema_version",
                detail: format!("unsupported schema version {feature_schema_version}, expected 1 or 2"),
            });
        }
        let d_in_emb = parse_usize_gt_zero(meta_map, "d_in_emb")?;
        let d_in_num = parse_usize_gt_zero(meta_map, "d_in_num")?;
        let d_e = parse_usize_gt_zero(meta_map, "d_e")?;
        let d_n = parse_usize_gt_zero(meta_map, "d_n")?;
        let d_model = parse_usize_gt_zero(meta_map, "d_model")?;
        let n_layers = parse_usize_gt_zero(meta_map, "n_layers")?;
        let tabm_k = parse_usize_gt_zero(meta_map, "tabm_k")?;
        let h_dim = parse_usize_gt_zero(meta_map, "h_dim")?;
        let h_actor_dim = parse_optional_usize_nonneg(meta_map, "h_actor_dim")?;
        let cell_type_str = meta_map
            .get("cell_type")
            .ok_or(NeuralError::MissingMetadata("cell_type"))?;
        let cell_type = CellType::from_metadata_str(cell_type_str)?;

        let temperature = parse_f32_positive(meta_map, "temperature")?;
        let cic_threshold = parse_f32_unit_interval(meta_map, "cic_threshold")?;
        let max_wait_ms = parse_f32_positive(meta_map, "max_wait_ms")?;

        let has_phase2_heads = parse_optional_bool(meta_map, "has_phase2_heads")?;
        let has_phase3_heads = parse_optional_bool(meta_map, "has_phase3_heads")?;
        let has_v22_heads = parse_optional_bool(meta_map, "has_v22_heads")?;
        let expected_embed_model_ver =
            parse_optional_nonempty_str(meta_map, "expected_embed_model_ver")?;
        let expected_prompt_regime_id =
            parse_optional_nonempty_str(meta_map, "expected_prompt_regime_id")?;

        // Infer or validate plr_n_frequencies from stem.plr.freqs.
        let freqs_view = tensors
            .tensor("stem.plr.freqs")
            .map_err(|_| NeuralError::MissingTensor("stem.plr.freqs".to_owned()))?;
        if freqs_view.shape().len() != 2 || freqs_view.shape()[0] != d_in_num || freqs_view.shape()[1] == 0 {
            return Err(NeuralError::ShapeMismatch {
                tensor: "stem.plr.freqs".to_owned(),
                expected: vec![d_in_num, 1],
                actual: freqs_view.shape().to_vec(),
            });
        }
        let plr_n_frequencies = freqs_view.shape()[1];
        if meta_map.contains_key("plr_n_frequencies") {
            let declared_freq = parse_usize_gt_zero(meta_map, "plr_n_frequencies")?;
            if declared_freq != plr_n_frequencies {
                return Err(NeuralError::InvalidMetadata {
                    key: "plr_n_frequencies",
                    detail: format!(
                        "declared {declared_freq} does not match stem.plr.freqs shape[1] ({plr_n_frequencies})"
                    ),
                });
            }
        }

        // Infer or validate d_ff from trunk.blocks.0.w_gate.weight.
        let gate0_view = tensors
            .tensor("trunk.blocks.0.w_gate.weight")
            .map_err(|_| NeuralError::MissingTensor("trunk.blocks.0.w_gate.weight".to_owned()))?;
        if gate0_view.shape().len() != 2 || gate0_view.shape()[1] != d_model || gate0_view.shape()[0] == 0 {
            return Err(NeuralError::ShapeMismatch {
                tensor: "trunk.blocks.0.w_gate.weight".to_owned(),
                expected: vec![1, d_model],
                actual: gate0_view.shape().to_vec(),
            });
        }
        let d_ff = gate0_view.shape()[0];
        if meta_map.contains_key("d_ff") {
            let declared_dff = parse_usize_gt_zero(meta_map, "d_ff")?;
            if declared_dff != d_ff {
                return Err(NeuralError::InvalidMetadata {
                    key: "d_ff",
                    detail: format!(
                        "declared {declared_dff} does not match trunk.blocks.0.w_gate.weight shape[0] ({d_ff})"
                    ),
                });
            }
        }

        let config = NeuralModelConfig {
            feature_schema_version,
            d_in_emb,
            d_in_num,
            plr_n_frequencies,
            d_e,
            d_n,
            d_model,
            d_ff,
            n_layers,
            tabm_k,
            h_dim,
            h_actor_dim,
            cell_type,
            temperature,
            cic_threshold,
            max_wait_ms,
            has_phase2_heads,
            has_phase3_heads,
            has_v22_heads,
            expected_embed_model_ver,
            expected_prompt_regime_id,
        };

        // Load & validate stem tensors.
        let emb_linear_weight = load_f32_tensor(&tensors, "stem.emb.linear.weight", &[d_e, d_in_emb])?;
        let emb_linear_bias = load_f32_tensor(&tensors, "stem.emb.linear.bias", &[d_e])?;
        let emb_norm_weight = load_f32_tensor(&tensors, "stem.emb.norm.weight", &[d_e])?;
        let emb_norm_bias = load_f32_tensor(&tensors, "stem.emb.norm.bias", &[d_e])?;
        let plr_freqs = load_f32_tensor(&tensors, "stem.plr.freqs", &[d_in_num, plr_n_frequencies])?;
        let plr_in_dim = d_in_num * 2 * plr_n_frequencies;
        let plr_linear_weight = load_f32_tensor(&tensors, "stem.plr.linear.weight", &[d_n, plr_in_dim])?;
        let plr_linear_bias = load_f32_tensor(&tensors, "stem.plr.linear.bias", &[d_n])?;
        let fusion_in_dim = d_e + d_n + h_dim + h_actor_dim;
        let fusion_weight = load_f32_tensor(&tensors, "stem.fusion.weight", &[d_model, fusion_in_dim])?;
        let fusion_bias = load_f32_tensor(&tensors, "stem.fusion.bias", &[d_model])?;

        // Load & validate trunk blocks.
        let tabm_in_scale = load_f32_tensor(&tensors, "trunk.tabm_in_scale", &[tabm_k, d_model])?;
        let mut blocks = Vec::with_capacity(n_layers);
        for l in 0..n_layers {
            blocks.push(SwiGluBlockWeights {
                norm_weight: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.norm.weight"), &[d_model])?,
                norm_bias: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.norm.bias"), &[d_model])?,
                w_gate: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.w_gate.weight"), &[d_ff, d_model])?,
                w_up: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.w_up.weight"), &[d_ff, d_model])?,
                w_down: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.w_down.weight"), &[d_model, d_ff])?,
                tabm_r: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.tabm_r"), &[tabm_k, d_model])?,
                tabm_s: load_f32_tensor(&tensors, &format!("trunk.blocks.{l}.tabm_s"), &[tabm_k, d_model])?,
            });
        }
        let final_norm_weight = load_f32_tensor(&tensors, "trunk.final_norm.weight", &[d_model])?;
        let final_norm_bias = load_f32_tensor(&tensors, "trunk.final_norm.bias", &[d_model])?;

        // Load & validate per-place working memory cell.
        let memory_cell = load_recurrent_cell(&tensors, "memory", cell_type, h_dim, d_model)?;

        // Load & validate optional per-actor working memory cell (when h_actor_dim > 0).
        let actor_memory_cell = if h_actor_dim > 0 {
            Some(load_recurrent_cell(
                &tensors,
                "actor_memory",
                cell_type,
                h_actor_dim,
                d_model,
            )?)
        } else {
            None
        };

        // Load & validate Phase 1 required heads.
        let gate_4_weight = load_f32_tensor(&tensors, "head.gate_4.weight", &[4, d_model])?;
        let gate_4_bias_vec = load_f32_tensor(&tensors, "head.gate_4.bias", &[4])?;
        let gate_4_bias = [
            gate_4_bias_vec[0],
            gate_4_bias_vec[1],
            gate_4_bias_vec[2],
            gate_4_bias_vec[3],
        ];
        let wait_delta_t_weight = load_f32_tensor(&tensors, "head.wait_delta_t.weight", &[1, d_model])?;
        let wait_delta_t_bias = load_f32_tensor(&tensors, "head.wait_delta_t.bias", &[1])?[0];

        // Load & validate optional Phase 2 & 3 heads.
        let phase2 = if has_phase2_heads {
            Some(Phase2HeadWeights {
                p_uncertainty_weight: load_f32_tensor(&tensors, "head.p_uncertainty.weight", &[1, d_model])?,
                p_uncertainty_bias: load_f32_tensor(&tensors, "head.p_uncertainty.bias", &[1])?[0],
                p_verify_weight: load_f32_tensor(&tensors, "head.p_verify.weight", &[1, d_model])?,
                p_verify_bias: load_f32_tensor(&tensors, "head.p_verify.bias", &[1])?[0],
            })
        } else {
            None
        };

        let phase3 = if has_phase3_heads {
            Some(Phase3HeadWeights {
                p_encode_importance_weight: load_f32_tensor(
                    &tensors,
                    "head.p_encode_importance.weight",
                    &[1, d_model],
                )?,
                p_encode_importance_bias: load_f32_tensor(
                    &tensors,
                    "head.p_encode_importance.bias",
                    &[1],
                )?[0],
                p_memory_relevance_weight: load_f32_tensor(
                    &tensors,
                    "head.p_memory_relevance.weight",
                    &[1, d_model],
                )?,
                p_memory_relevance_bias: load_f32_tensor(
                    &tensors,
                    "head.p_memory_relevance.bias",
                    &[1],
                )?[0],
            })
        } else {
            None
        };

        let v22_heads = if has_v22_heads {
            let compute_tier_weight =
                load_f32_tensor(&tensors, "head.compute_tier.weight", &[3, d_model])?;
            let ct_bias_vec = load_f32_tensor(&tensors, "head.compute_tier.bias", &[3])?;
            let compute_tier_bias = [ct_bias_vec[0], ct_bias_vec[1], ct_bias_vec[2]];
            let p_user_correction_weight =
                load_f32_tensor(&tensors, "head.p_user_correction.weight", &[1, d_model])?;
            let p_user_correction_bias =
                load_f32_tensor(&tensors, "head.p_user_correction.bias", &[1])?[0];
            Some(V22HeadWeights {
                compute_tier_weight,
                compute_tier_bias,
                p_user_correction_weight,
                p_user_correction_bias,
            })
        } else {
            None
        };

        Ok(Self {
            config,
            blake3: actual_blake3,
            emb_linear_weight,
            emb_linear_bias,
            emb_norm_weight,
            emb_norm_bias,
            plr_freqs,
            plr_linear_weight,
            plr_linear_bias,
            fusion_weight,
            fusion_bias,
            tabm_in_scale,
            blocks,
            final_norm_weight,
            final_norm_bias,
            memory_cell,
            actor_memory_cell,
            gate_4_weight,
            gate_4_bias,
            wait_delta_t_weight,
            wait_delta_t_bias,
            phase2,
            phase3,
            v22_heads,
        })
    }

    /// Returns the total scalar parameter count across all loaded tensors.
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        let stem = self.emb_linear_weight.len()
            + self.emb_linear_bias.len()
            + self.emb_norm_weight.len()
            + self.emb_norm_bias.len()
            + self.plr_freqs.len()
            + self.plr_linear_weight.len()
            + self.plr_linear_bias.len()
            + self.fusion_weight.len()
            + self.fusion_bias.len();
        let trunk: usize = self.tabm_in_scale.len()
            + self
                .blocks
                .iter()
                .map(|b| {
                    b.norm_weight.len()
                        + b.norm_bias.len()
                        + b.w_gate.len()
                        + b.w_up.len()
                        + b.w_down.len()
                        + b.tabm_r.len()
                        + b.tabm_s.len()
                })
                .sum::<usize>()
            + self.final_norm_weight.len()
            + self.final_norm_bias.len();
        let cell = recurrent_cell_param_count(&self.memory_cell);
        let actor_cell = self
            .actor_memory_cell
            .as_ref()
            .map_or(0, recurrent_cell_param_count);
        let p1 = self.gate_4_weight.len() + 4 + self.wait_delta_t_weight.len() + 1;
        let p2 = self.phase2.as_ref().map_or(0, |h| {
            h.p_uncertainty_weight.len() + 1 + h.p_verify_weight.len() + 1
        });
        let p3 = self.phase3.as_ref().map_or(0, |h| {
            h.p_encode_importance_weight.len() + 1 + h.p_memory_relevance_weight.len() + 1
        });
        let v22 = self.v22_heads.as_ref().map_or(0, |h| {
            h.compute_tier_weight.len() + 3 + h.p_user_correction_weight.len() + 1
        });
        stem + trunk + cell + actor_cell + p1 + p2 + p3 + v22
    }
}

fn load_recurrent_cell(
    tensors: &SafeTensors<'_>,
    prefix: &str,
    cell_type: CellType,
    h_dim: usize,
    d_model: usize,
) -> Result<RecurrentCellWeights, NeuralError> {
    match cell_type {
        CellType::MinGru => Ok(RecurrentCellWeights::MinGru {
            w_z_weight: load_f32_tensor(tensors, &format!("{prefix}.mingru.w_z.weight"), &[h_dim, d_model])?,
            w_z_bias: load_f32_tensor(tensors, &format!("{prefix}.mingru.w_z.bias"), &[h_dim])?,
            w_h_weight: load_f32_tensor(tensors, &format!("{prefix}.mingru.w_h.weight"), &[h_dim, d_model])?,
            w_h_bias: load_f32_tensor(tensors, &format!("{prefix}.mingru.w_h.bias"), &[h_dim])?,
        }),
        CellType::GatedDeltaNet2 => Ok(RecurrentCellWeights::GatedDeltaNet2 {
            w_erase_weight: load_f32_tensor(tensors, &format!("{prefix}.gdn2.w_erase.weight"), &[h_dim, d_model])?,
            w_erase_bias: load_f32_tensor(tensors, &format!("{prefix}.gdn2.w_erase.bias"), &[h_dim])?,
            w_write_weight: load_f32_tensor(tensors, &format!("{prefix}.gdn2.w_write.weight"), &[h_dim, d_model])?,
            w_write_bias: load_f32_tensor(tensors, &format!("{prefix}.gdn2.w_write.bias"), &[h_dim])?,
            w_delta_weight: load_f32_tensor(tensors, &format!("{prefix}.gdn2.w_delta.weight"), &[h_dim, d_model])?,
            w_delta_bias: load_f32_tensor(tensors, &format!("{prefix}.gdn2.w_delta.bias"), &[h_dim])?,
        }),
    }
}

fn recurrent_cell_param_count(cell: &RecurrentCellWeights) -> usize {
    match cell {
        RecurrentCellWeights::MinGru {
            w_z_weight,
            w_z_bias,
            w_h_weight,
            w_h_bias,
        } => w_z_weight.len() + w_z_bias.len() + w_h_weight.len() + w_h_bias.len(),
        RecurrentCellWeights::GatedDeltaNet2 {
            w_erase_weight,
            w_erase_bias,
            w_write_weight,
            w_write_bias,
            w_delta_weight,
            w_delta_bias,
        } => {
            w_erase_weight.len()
                + w_erase_bias.len()
                + w_write_weight.len()
                + w_write_bias.len()
                + w_delta_weight.len()
                + w_delta_bias.len()
        }
    }
}

fn load_f32_tensor(
    tensors: &SafeTensors<'_>,
    name: &str,
    expected_shape: &[usize],
) -> Result<Vec<f32>, NeuralError> {
    let view = tensors
        .tensor(name)
        .map_err(|_| NeuralError::MissingTensor(name.to_owned()))?;
    if view.dtype() != Dtype::F32 {
        return Err(NeuralError::InvalidDtype {
            tensor: name.to_owned(),
            detail: format!("expected F32, got {:?}", view.dtype()),
        });
    }
    if view.shape() != expected_shape {
        return Err(NeuralError::ShapeMismatch {
            tensor: name.to_owned(),
            expected: expected_shape.to_vec(),
            actual: view.shape().to_vec(),
        });
    }
    let expected_elems: usize = expected_shape.iter().product();
    let raw = view.data();
    if raw.len() != expected_elems * 4 {
        return Err(NeuralError::InvalidDtype {
            tensor: name.to_owned(),
            detail: format!(
                "expected {} bytes for F32 shape {:?}, got {}",
                expected_elems * 4,
                expected_shape,
                raw.len()
            ),
        });
    }
    let mut out = Vec::with_capacity(expected_elems);
    for (idx, chunk) in raw.as_chunks::<4>().0.iter().enumerate() {
        let val = f32::from_le_bytes(*chunk);
        if !val.is_finite() {
            return Err(NeuralError::NonFiniteWeight {
                tensor: name.to_owned(),
                index: idx,
            });
        }
        out.push(val);
    }
    Ok(out)
}

fn parse_u16_gt_zero(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<u16, NeuralError> {
    let raw = meta.get(key).ok_or(NeuralError::MissingMetadata(key))?;
    let val = raw
        .parse::<u16>()
        .map_err(|e| NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' is not a valid u16: {e}"),
        })?;
    if val == 0 {
        return Err(NeuralError::InvalidMetadata {
            key,
            detail: "must be > 0".to_owned(),
        });
    }
    Ok(val)
}

fn parse_usize_gt_zero(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<usize, NeuralError> {
    let raw = meta.get(key).ok_or(NeuralError::MissingMetadata(key))?;
    let val = raw
        .parse::<usize>()
        .map_err(|e| NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' is not a valid positive integer: {e}"),
        })?;
    if val == 0 {
        return Err(NeuralError::InvalidMetadata {
            key,
            detail: "must be > 0".to_owned(),
        });
    }
    Ok(val)
}

fn parse_f32_positive(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<f32, NeuralError> {
    let raw = meta.get(key).ok_or(NeuralError::MissingMetadata(key))?;
    let val = raw
        .parse::<f32>()
        .map_err(|e| NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' is not a valid f32: {e}"),
        })?;
    if !val.is_finite() || val <= 0.0 {
        return Err(NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' must be finite and > 0.0"),
        });
    }
    Ok(val)
}

fn parse_f32_unit_interval(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<f32, NeuralError> {
    let raw = meta.get(key).ok_or(NeuralError::MissingMetadata(key))?;
    let val = raw
        .parse::<f32>()
        .map_err(|e| NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' is not a valid f32: {e}"),
        })?;
    if !val.is_finite() || !(0.0..=1.0).contains(&val) {
        return Err(NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' must be finite and in [0.0, 1.0]"),
        });
    }
    Ok(val)
}

fn parse_optional_nonempty_str(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<Option<String>, NeuralError> {
    let Some(raw) = meta.get(key) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(NeuralError::InvalidMetadata {
            key,
            detail: "must be non-empty when present".to_owned(),
        });
    }
    Ok(Some(trimmed.to_owned()))
}

fn parse_optional_usize_nonneg(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<usize, NeuralError> {
    let Some(raw) = meta.get(key) else {
        return Ok(0);
    };
    raw.parse::<usize>()
        .map_err(|e| NeuralError::InvalidMetadata {
            key,
            detail: format!("'{raw}' is not a valid non-negative integer: {e}"),
        })
}

fn parse_optional_bool(
    meta: &HashMap<String, String>,
    key: &'static str,
) -> Result<bool, NeuralError> {
    let Some(raw) = meta.get(key) else {
        return Ok(false);
    };
    match raw.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(NeuralError::InvalidMetadata {
            key,
            detail: format!("'{other}' must be 'true' or 'false'"),
        }),
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::collections::HashMap;

    use safetensors::tensor::{Dtype, TensorView};

    use super::CellType;

    struct TensorEntry {
        shape: Vec<usize>,
        bytes: Vec<u8>,
    }

    fn make_f32_data(len: usize, seed: u64, base_bias: f32) -> Vec<u8> {
        let mut out = Vec::with_capacity(len * 4);
        let mut s = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        for i in 0..len {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let unit = ((s >> 40) as f32) / ((1u32 << 24) as f32) - 0.5;
            let val = base_bias + unit * 0.08 + ((i % 7) as f32 - 3.0) * 0.002;
            out.extend_from_slice(&val.to_le_bytes());
        }
        out
    }

    fn make_ones(len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len * 4);
        for _ in 0..len {
            out.extend_from_slice(&1.0_f32.to_le_bytes());
        }
        out
    }

    pub(crate) type RawSyntheticTensors =
        (HashMap<String, String>, Vec<(String, Vec<usize>, Vec<u8>)>);

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_synthetic_tensors_map(
        d_in_emb: usize,
        d_in_num: usize,
        d_e: usize,
        d_n: usize,
        d_model: usize,
        d_ff: usize,
        n_layers: usize,
        tabm_k: usize,
        h_dim: usize,
        n_freq: usize,
        cell_type: CellType,
        has_phase2: bool,
        has_phase3: bool,
    ) -> RawSyntheticTensors {
        let mut meta = HashMap::new();
        meta.insert("feature_schema_version".to_owned(), "1".to_owned());
        meta.insert("d_in_emb".to_owned(), d_in_emb.to_string());
        meta.insert("d_in_num".to_owned(), d_in_num.to_string());
        meta.insert("plr_n_frequencies".to_owned(), n_freq.to_string());
        meta.insert("d_e".to_owned(), d_e.to_string());
        meta.insert("d_n".to_owned(), d_n.to_string());
        meta.insert("d_model".to_owned(), d_model.to_string());
        meta.insert("d_ff".to_owned(), d_ff.to_string());
        meta.insert("n_layers".to_owned(), n_layers.to_string());
        meta.insert("tabm_k".to_owned(), tabm_k.to_string());
        meta.insert("h_dim".to_owned(), h_dim.to_string());
        meta.insert("cell_type".to_owned(), cell_type.as_str().to_owned());
        meta.insert("temperature".to_owned(), "1.25".to_owned());
        meta.insert("cic_threshold".to_owned(), "0.55".to_owned());
        meta.insert("max_wait_ms".to_owned(), "30000.0".to_owned());
        meta.insert("has_phase2_heads".to_owned(), has_phase2.to_string());
        meta.insert("has_phase3_heads".to_owned(), has_phase3.to_string());

        let mut list: Vec<(String, Vec<usize>, Vec<u8>)> = Vec::new();
        let mut push = |name: &str, shape: Vec<usize>, bytes: Vec<u8>| {
            list.push((name.to_owned(), shape, bytes));
        };

        push("stem.emb.linear.weight", vec![d_e, d_in_emb], make_f32_data(d_e * d_in_emb, 1, 0.0));
        push("stem.emb.linear.bias", vec![d_e], make_f32_data(d_e, 2, 0.0));
        push("stem.emb.norm.weight", vec![d_e], make_ones(d_e));
        push("stem.emb.norm.bias", vec![d_e], make_f32_data(d_e, 3, 0.0));
        push("stem.plr.freqs", vec![d_in_num, n_freq], make_f32_data(d_in_num * n_freq, 4, 0.5));
        push(
            "stem.plr.linear.weight",
            vec![d_n, d_in_num * 2 * n_freq],
            make_f32_data(d_n * d_in_num * 2 * n_freq, 5, 0.0),
        );
        push("stem.plr.linear.bias", vec![d_n], make_f32_data(d_n, 6, 0.0));
        push(
            "stem.fusion.weight",
            vec![d_model, d_e + d_n + h_dim],
            make_f32_data(d_model * (d_e + d_n + h_dim), 7, 0.0),
        );
        push("stem.fusion.bias", vec![d_model], make_f32_data(d_model, 8, 0.0));

        push("trunk.tabm_in_scale", vec![tabm_k, d_model], make_f32_data(tabm_k * d_model, 9, 1.0));
        for l in 0..n_layers {
            let s = 100 + (l as u64) * 20;
            push(&format!("trunk.blocks.{l}.norm.weight"), vec![d_model], make_ones(d_model));
            push(&format!("trunk.blocks.{l}.norm.bias"), vec![d_model], make_f32_data(d_model, s + 1, 0.0));
            push(&format!("trunk.blocks.{l}.w_gate.weight"), vec![d_ff, d_model], make_f32_data(d_ff * d_model, s + 2, 0.0));
            push(&format!("trunk.blocks.{l}.w_up.weight"), vec![d_ff, d_model], make_f32_data(d_ff * d_model, s + 3, 0.0));
            push(&format!("trunk.blocks.{l}.w_down.weight"), vec![d_model, d_ff], make_f32_data(d_model * d_ff, s + 4, 0.0));
            push(&format!("trunk.blocks.{l}.tabm_r"), vec![tabm_k, d_model], make_f32_data(tabm_k * d_model, s + 5, 1.0));
            push(&format!("trunk.blocks.{l}.tabm_s"), vec![tabm_k, d_model], make_f32_data(tabm_k * d_model, s + 6, 1.0));
        }
        push("trunk.final_norm.weight", vec![d_model], make_ones(d_model));
        push("trunk.final_norm.bias", vec![d_model], make_f32_data(d_model, 20, 0.0));

        match cell_type {
            CellType::MinGru => {
                push("memory.mingru.w_z.weight", vec![h_dim, d_model], make_f32_data(h_dim * d_model, 30, 0.0));
                push("memory.mingru.w_z.bias", vec![h_dim], make_f32_data(h_dim, 31, 0.0));
                push("memory.mingru.w_h.weight", vec![h_dim, d_model], make_f32_data(h_dim * d_model, 32, 0.0));
                push("memory.mingru.w_h.bias", vec![h_dim], make_f32_data(h_dim, 33, 0.0));
            }
            CellType::GatedDeltaNet2 => {
                push("memory.gdn2.w_erase.weight", vec![h_dim, d_model], make_f32_data(h_dim * d_model, 34, 0.0));
                push("memory.gdn2.w_erase.bias", vec![h_dim], make_f32_data(h_dim, 35, 0.0));
                push("memory.gdn2.w_write.weight", vec![h_dim, d_model], make_f32_data(h_dim * d_model, 36, 0.0));
                push("memory.gdn2.w_write.bias", vec![h_dim], make_f32_data(h_dim, 37, 0.0));
                push("memory.gdn2.w_delta.weight", vec![h_dim, d_model], make_f32_data(h_dim * d_model, 38, 0.0));
                push("memory.gdn2.w_delta.bias", vec![h_dim], make_f32_data(h_dim, 39, 0.0));
            }
        }

        push("head.gate_4.weight", vec![4, d_model], make_f32_data(4 * d_model, 40, 0.0));
        push("head.gate_4.bias", vec![4], make_f32_data(4, 41, 0.0));
        push("head.wait_delta_t.weight", vec![1, d_model], make_f32_data(d_model, 42, 0.0));
        push("head.wait_delta_t.bias", vec![1], make_f32_data(1, 43, 0.0));

        if has_phase2 {
            push("head.p_uncertainty.weight", vec![1, d_model], make_f32_data(d_model, 50, 0.0));
            push("head.p_uncertainty.bias", vec![1], make_f32_data(1, 51, 0.0));
            push("head.p_verify.weight", vec![1, d_model], make_f32_data(d_model, 52, 0.0));
            push("head.p_verify.bias", vec![1], make_f32_data(1, 53, 0.0));
        }

        if has_phase3 {
            push("head.p_encode_importance.weight", vec![1, d_model], make_f32_data(d_model, 60, 0.0));
            push("head.p_encode_importance.bias", vec![1], make_f32_data(1, 61, 0.0));
            push("head.p_memory_relevance.weight", vec![1, d_model], make_f32_data(d_model, 62, 0.0));
            push("head.p_memory_relevance.bias", vec![1], make_f32_data(1, 63, 0.0));
        }

        (meta, list)
    }

    pub(crate) fn serialize_tensors_map(
        meta: HashMap<String, String>,
        entries: Vec<(String, Vec<usize>, Vec<u8>)>,
    ) -> Vec<u8> {
        let mut owned: Vec<(String, TensorEntry)> = entries
            .into_iter()
            .map(|(k, shape, bytes)| (k, TensorEntry { shape, bytes }))
            .collect();
        let views: Vec<(&str, TensorView<'_>)> = owned
            .iter_mut()
            .map(|(name, e)| {
                let tv = TensorView::new(Dtype::F32, e.shape.clone(), &e.bytes).expect("valid view");
                (name.as_str(), tv)
            })
            .collect();
        safetensors::serialize(views, Some(meta)).expect("serialize safetensors")
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_synthetic_safetensors(
        d_in_emb: usize,
        d_in_num: usize,
        d_e: usize,
        d_n: usize,
        d_model: usize,
        d_ff: usize,
        n_layers: usize,
        tabm_k: usize,
        h_dim: usize,
        n_freq: usize,
        cell_type: CellType,
        has_phase2: bool,
        has_phase3: bool,
    ) -> Vec<u8> {
        let (meta, entries) = build_synthetic_tensors_map(
            d_in_emb,
            d_in_num,
            d_e,
            d_n,
            d_model,
            d_ff,
            n_layers,
            tabm_k,
            h_dim,
            n_freq,
            cell_type,
            has_phase2,
            has_phase3,
        );
        serialize_tensors_map(meta, entries)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_synthetic_v22_tensors_map(
        schema_version: u16,
        d_in_emb: usize,
        d_in_num: usize,
        d_e: usize,
        d_n: usize,
        d_model: usize,
        d_ff: usize,
        n_layers: usize,
        tabm_k: usize,
        h_dim: usize,
        h_actor_dim: usize,
        n_freq: usize,
        cell_type: CellType,
        has_phase2: bool,
        has_phase3: bool,
        has_v22_heads: bool,
    ) -> RawSyntheticTensors {
        let (mut meta, mut list) = build_synthetic_tensors_map(
            d_in_emb,
            d_in_num,
            d_e,
            d_n,
            d_model,
            d_ff,
            n_layers,
            tabm_k,
            h_dim,
            n_freq,
            cell_type,
            has_phase2,
            has_phase3,
        );
        meta.insert("feature_schema_version".to_owned(), schema_version.to_string());
        meta.insert("h_actor_dim".to_owned(), h_actor_dim.to_string());
        meta.insert("has_v22_heads".to_owned(), has_v22_heads.to_string());

        if h_actor_dim > 0 {
            let fusion_in = d_e + d_n + h_dim + h_actor_dim;
            if let Some((_, shape, bytes)) =
                list.iter_mut().find(|(n, _, _)| n == "stem.fusion.weight")
            {
                *shape = vec![d_model, fusion_in];
                *bytes = make_f32_data(d_model * fusion_in, 77, 0.0);
            }
            match cell_type {
                CellType::MinGru => {
                    list.push((
                        "actor_memory.mingru.w_z.weight".to_owned(),
                        vec![h_actor_dim, d_model],
                        make_f32_data(h_actor_dim * d_model, 70, 0.0),
                    ));
                    list.push((
                        "actor_memory.mingru.w_z.bias".to_owned(),
                        vec![h_actor_dim],
                        make_f32_data(h_actor_dim, 71, 0.0),
                    ));
                    list.push((
                        "actor_memory.mingru.w_h.weight".to_owned(),
                        vec![h_actor_dim, d_model],
                        make_f32_data(h_actor_dim * d_model, 72, 0.0),
                    ));
                    list.push((
                        "actor_memory.mingru.w_h.bias".to_owned(),
                        vec![h_actor_dim],
                        make_f32_data(h_actor_dim, 73, 0.0),
                    ));
                }
                CellType::GatedDeltaNet2 => {
                    list.push((
                        "actor_memory.gdn2.w_erase.weight".to_owned(),
                        vec![h_actor_dim, d_model],
                        make_f32_data(h_actor_dim * d_model, 74, 0.0),
                    ));
                    list.push((
                        "actor_memory.gdn2.w_erase.bias".to_owned(),
                        vec![h_actor_dim],
                        make_f32_data(h_actor_dim, 75, 0.0),
                    ));
                    list.push((
                        "actor_memory.gdn2.w_write.weight".to_owned(),
                        vec![h_actor_dim, d_model],
                        make_f32_data(h_actor_dim * d_model, 76, 0.0),
                    ));
                    list.push((
                        "actor_memory.gdn2.w_write.bias".to_owned(),
                        vec![h_actor_dim],
                        make_f32_data(h_actor_dim, 77, 0.0),
                    ));
                    list.push((
                        "actor_memory.gdn2.w_delta.weight".to_owned(),
                        vec![h_actor_dim, d_model],
                        make_f32_data(h_actor_dim * d_model, 78, 0.0),
                    ));
                    list.push((
                        "actor_memory.gdn2.w_delta.bias".to_owned(),
                        vec![h_actor_dim],
                        make_f32_data(h_actor_dim, 79, 0.0),
                    ));
                }
            }
        }

        if has_v22_heads {
            list.push((
                "head.compute_tier.weight".to_owned(),
                vec![3, d_model],
                make_f32_data(3 * d_model, 80, 0.0),
            ));
            list.push((
                "head.compute_tier.bias".to_owned(),
                vec![3],
                make_f32_data(3, 81, 0.0),
            ));
            list.push((
                "head.p_user_correction.weight".to_owned(),
                vec![1, d_model],
                make_f32_data(d_model, 82, 0.0),
            ));
            list.push((
                "head.p_user_correction.bias".to_owned(),
                vec![1],
                make_f32_data(1, 83, 0.0),
            ));
        }

        (meta, list)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_synthetic_v22_safetensors(
        schema_version: u16,
        d_in_emb: usize,
        d_in_num: usize,
        d_e: usize,
        d_n: usize,
        d_model: usize,
        d_ff: usize,
        n_layers: usize,
        tabm_k: usize,
        h_dim: usize,
        h_actor_dim: usize,
        n_freq: usize,
        cell_type: CellType,
        has_phase2: bool,
        has_phase3: bool,
        has_v22_heads: bool,
    ) -> Vec<u8> {
        let (meta, entries) = build_synthetic_v22_tensors_map(
            schema_version,
            d_in_emb,
            d_in_num,
            d_e,
            d_n,
            d_model,
            d_ff,
            n_layers,
            tabm_k,
            h_dim,
            h_actor_dim,
            n_freq,
            cell_type,
            has_phase2,
            has_phase3,
            has_v22_heads,
        );
        serialize_tensors_map(meta, entries)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{
        build_synthetic_safetensors, build_synthetic_tensors_map, serialize_tensors_map,
    };
    use super::*;

    #[test]
    fn loads_valid_mingru_and_gated_deltanet2_models_with_blake3() {
        for cell in [CellType::MinGru, CellType::GatedDeltaNet2] {
            let bytes = build_synthetic_safetensors(
                256, 18, 64, 32, 128, 192, 2, 8, 64, 8, cell, true, false,
            );
            let expected = blake3::hash(&bytes);
            let model = NeuralModel::from_safetensors_bytes(&bytes, Some(expected))
                .expect("valid model should load");
            assert_eq!(model.blake3, expected);
            assert_eq!(model.config.cell_type, cell);
            assert!(model.phase2.is_some());
            assert!(model.phase3.is_none());
            assert!(model.parameter_count() > 140_000);
        }
    }

    #[test]
    fn rejects_blake3_mismatch() {
        let bytes = build_synthetic_safetensors(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::MinGru,
            false,
            false,
        );
        let wrong_digest = [0xAA_u8; 32];
        let err = NeuralModel::from_safetensors_bytes(&bytes, Some(wrong_digest))
            .expect_err("mismatched digest must fail closed");
        assert!(matches!(err, NeuralError::Blake3Mismatch { .. }));
    }

    #[test]
    fn rejects_missing_tensor_and_corrupt_container() {
        let err_corrupt = NeuralModel::from_safetensors_bytes(b"not-a-safetensor", None)
            .expect_err("corrupt bytes must fail");
        assert!(matches!(err_corrupt, NeuralError::SafeTensorParse(_)));

        let (meta, mut entries) = build_synthetic_tensors_map(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::GatedDeltaNet2,
            false,
            false,
        );
        entries.retain(|(name, _, _)| name != "memory.gdn2.w_write.weight");
        let bytes = serialize_tensors_map(meta, entries);
        let err_missing =
            NeuralModel::from_safetensors_bytes(&bytes, None).expect_err("missing tensor must fail");
        assert_eq!(
            err_missing,
            NeuralError::MissingTensor("memory.gdn2.w_write.weight".to_owned())
        );
    }

    #[test]
    fn rejects_non_finite_weights_and_metadata() {
        // Non-finite tensor value (NaN)
        let (meta, mut entries) = build_synthetic_tensors_map(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::MinGru,
            false,
            false,
        );
        if let Some((_, _, bytes)) = entries.iter_mut().find(|(n, _, _)| n == "head.gate_4.bias") {
            let nan_bytes = f32::NAN.to_le_bytes();
            bytes[4..8].copy_from_slice(&nan_bytes);
        }
        let nan_model = serialize_tensors_map(meta, entries);
        let err_nan = NeuralModel::from_safetensors_bytes(&nan_model, None)
            .expect_err("NaN weight must be rejected");
        assert_eq!(
            err_nan,
            NeuralError::NonFiniteWeight {
                tensor: "head.gate_4.bias".to_owned(),
                index: 1,
            }
        );

        // Non-finite metadata scalar (Inf temperature)
        let (mut meta_inf, entries_inf) = build_synthetic_tensors_map(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::MinGru,
            false,
            false,
        );
        meta_inf.insert("temperature".to_owned(), "inf".to_owned());
        let inf_model = serialize_tensors_map(meta_inf, entries_inf);
        let err_inf = NeuralModel::from_safetensors_bytes(&inf_model, None)
            .expect_err("Inf temperature must be rejected");
        assert!(matches!(
            err_inf,
            NeuralError::InvalidMetadata {
                key: "temperature",
                ..
            }
        ));
    }

    #[test]
    fn rejects_shape_mismatch() {
        let (meta, mut entries) = build_synthetic_tensors_map(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::MinGru,
            false,
            false,
        );
        if let Some((_, shape, bytes)) = entries.iter_mut().find(|(n, _, _)| n == "head.gate_4.bias")
        {
            *shape = vec![3];
            bytes.truncate(12);
        }
        let bad_shape = serialize_tensors_map(meta, entries);
        let err = NeuralModel::from_safetensors_bytes(&bad_shape, None)
            .expect_err("shape mismatch must fail");
        assert_eq!(
            err,
            NeuralError::ShapeMismatch {
                tensor: "head.gate_4.bias".to_owned(),
                expected: vec![4],
                actual: vec![3],
            }
        );
    }

    #[test]
    fn parses_optional_embedding_contract_metadata_and_keeps_v1_compat() {
        let (mut meta, entries) = build_synthetic_tensors_map(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::GatedDeltaNet2,
            true,
            true,
        );
        // Without optional keys -> None (legacy fixture compatibility)
        let bytes_legacy = serialize_tensors_map(meta.clone(), entries.clone());
        let m_legacy = NeuralModel::from_safetensors_bytes(&bytes_legacy, None).expect("legacy");
        assert_eq!(m_legacy.config.expected_embed_model_ver, None);
        assert_eq!(m_legacy.config.expected_prompt_regime_id, None);

        // With expected_embed_model_ver & expected_prompt_regime_id -> parsed cleanly
        meta.insert("expected_embed_model_ver".to_owned(), "egemma2-256".to_owned());
        meta.insert("expected_prompt_regime_id".to_owned(), "t_d".to_owned());
        let bytes_contract = serialize_tensors_map(meta, entries);
        let m_contract =
            NeuralModel::from_safetensors_bytes(&bytes_contract, None).expect("contract");
        assert_eq!(
            m_contract.config.expected_embed_model_ver.as_deref(),
            Some("egemma2-256")
        );
        assert_eq!(
            m_contract.config.expected_prompt_regime_id.as_deref(),
            Some("t_d")
        );
    }

    #[test]
    fn loads_v22_schema2_models_with_h_actor_and_v22_heads_for_both_cells() {
        for cell in [CellType::MinGru, CellType::GatedDeltaNet2] {
            for h_actor_dim in [0usize, 24, 32] {
                let bytes = tests_support::build_synthetic_v22_safetensors(
                    2,
                    256,
                    32,
                    64,
                    32,
                    128,
                    192,
                    2,
                    8,
                    64,
                    h_actor_dim,
                    8,
                    cell,
                    true,
                    true,
                    true,
                );
                let digest = blake3::hash(&bytes);
                let model = NeuralModel::from_safetensors_bytes(&bytes, Some(digest))
                    .expect("valid v2.2 model should load");
                assert_eq!(model.config.feature_schema_version, 2);
                assert_eq!(model.config.d_in_num, 32);
                assert_eq!(model.config.h_actor_dim, h_actor_dim);
                assert!(model.config.has_v22_heads);
                assert_eq!(model.actor_memory_cell.is_some(), h_actor_dim > 0);
                assert!(model.v22_heads.is_some());
                assert_eq!(
                    model.fusion_weight.len(),
                    128 * (64 + 32 + 64 + h_actor_dim)
                );
                assert!(model.parameter_count() > 140_000);
            }
        }
    }

    #[test]
    fn rejects_missing_v22_tensors_and_unsupported_schema_version() {
        // Missing compute_tier tensor when has_v22_heads == true
        let (meta, mut entries) = tests_support::build_synthetic_v22_tensors_map(
            2,
            256,
            32,
            64,
            32,
            128,
            192,
            2,
            4,
            64,
            24,
            8,
            CellType::MinGru,
            true,
            false,
            true,
        );
        entries.retain(|(n, _, _)| n != "head.compute_tier.weight");
        let err = NeuralModel::from_safetensors_bytes(&serialize_tensors_map(meta, entries), None)
            .expect_err("missing head.compute_tier.weight must fail");
        assert_eq!(
            err,
            NeuralError::MissingTensor("head.compute_tier.weight".to_owned())
        );

        // Missing actor_memory tensor when h_actor_dim > 0
        let (meta2, mut entries2) = tests_support::build_synthetic_v22_tensors_map(
            2,
            256,
            32,
            64,
            32,
            128,
            192,
            2,
            4,
            64,
            24,
            8,
            CellType::GatedDeltaNet2,
            false,
            false,
            false,
        );
        entries2.retain(|(n, _, _)| n != "actor_memory.gdn2.w_delta.weight");
        let err2 =
            NeuralModel::from_safetensors_bytes(&serialize_tensors_map(meta2, entries2), None)
                .expect_err("missing actor_memory tensor must fail");
        assert_eq!(
            err2,
            NeuralError::MissingTensor("actor_memory.gdn2.w_delta.weight".to_owned())
        );

        // Unsupported feature_schema_version = 3
        let (mut meta3, entries3) = tests_support::build_synthetic_v22_tensors_map(
            2,
            256,
            32,
            64,
            32,
            128,
            192,
            2,
            4,
            64,
            0,
            8,
            CellType::MinGru,
            false,
            false,
            false,
        );
        meta3.insert("feature_schema_version".to_owned(), "3".to_owned());
        let err3 =
            NeuralModel::from_safetensors_bytes(&serialize_tensors_map(meta3, entries3), None)
                .expect_err("unsupported schema version must fail");
        assert!(matches!(
            err3,
            NeuralError::InvalidMetadata {
                key: "feature_schema_version",
                ..
            }
        ));
    }
}
