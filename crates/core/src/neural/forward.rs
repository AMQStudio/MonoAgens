//! Pure-Rust `f32` forward pass for the Unified Neural Cognitive Controller.
//!
//! Executes `PLR` numeric embeddings, `LayerNorm`-projected frozen embedding stem,
//! Modality-Fusion concat, `TabM` `k`-member implicit-ensemble Pre-LN Residual SwiGLU
//! trunk (computing ensemble-mean probabilities and inter-member epistemic JSD
//! uncertainty), per-place `64d` recurrent working memory (`minGRU` or `Gated DeltaNet-2`),
//! bounded per-place Warm-Start UCB FiLM/logit adaptation, and phased output heads.

use core::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::neural::adapt::{AdaptConfig, PlaceAdaptState};
use crate::neural::weights::{NeuralModel, RecurrentCellWeights};
use crate::neural::{
    ComputeTier, DEFAULT_NUM_FEATURES_V1, DEFAULT_NUM_FEATURES_V2, GateClass, NeuralError,
};

const LAYER_NORM_EPS: f32 = 1e-5;
const LN_4: f32 = 1.386_294_4; // ln(4.0)

/// Complete output bundle produced by one neural forward step (or neutral fallback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeuralOutputs {
    /// Mean raw `gate_4` logits `[Silent, Note, Wait, Go]` (after any online adaptation).
    pub gate_logits: [f32; 4],
    /// Temperature-scaled ensemble-mean `gate_4` probabilities `[Silent, Note, Wait, Go]`.
    pub gate_probs: [f32; 4],
    /// Argmax gate class from `gate_probs`.
    pub predicted_class: GateClass,
    /// Bounded wait horizon in milliseconds (`sigmoid(wait_logit) * max_wait_ms`).
    pub wait_delta_ms: f32,
    /// Normalized `TabM` inter-member epistemic disagreement (JSD / ln(4)) in `[0.0, 1.0]`.
    pub epistemic_uncertainty: f32,
    /// Finite-sample CIC abstention flag (`max(gate_probs) < cic_threshold`).
    pub cic_abstain: bool,
    /// Updated per-place recurrent working memory vector (`h_t`).
    pub h_next: Vec<f32>,
    /// Updated per-actor recurrent working memory vector (`h_actor_t`); empty when `h_actor_dim == 0`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub h_actor_next: Vec<f32>,
    /// Optional Phase 2 uncertainty scalar (head sigmoid + epistemic spread, clamped to `[0, 1]`).
    pub p_uncertainty: Option<f32>,
    /// Optional Phase 2 verification / context-depth hint in `[0.0, 1.0]`.
    pub p_verify: Option<f32>,
    /// Optional Phase 3 memory write gate importance in `[0.0, 1.0]`.
    pub p_encode_importance: Option<f32>,
    /// Optional Phase 3 memory retrieval relevance in `[0.0, 1.0]`.
    pub p_memory_relevance: Option<f32>,
    /// Optional V2.2 compute-tier softmax probabilities `[Fast, Standard, Deep]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_tier_probs: Option<[f32; 3]>,
    /// Optional V2.2 compute tier after deterministic ODAR epistemic escalation floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_tier: Option<ComputeTier>,
    /// Optional V2.2 calibrated probability that the percept corrects the soul's prior turn (`[0, 1]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_user_correction: Option<f32>,
    /// `BLAKE3` digest of the model artifact (`None` when running in neutral fallback).
    pub model_blake3: Option<[u8; 32]>,
    /// Feature schema version declared by the model (`0` in neutral fallback).
    pub feature_schema_version: u16,
    /// `true` when `model` was `None` and neutral fallback outputs were returned.
    pub is_neutral_fallback: bool,
    /// `true` when per-place online adaptation was active and applied.
    pub online_adaptation_applied: bool,
}

/// Deterministic neutral fallback outputs when no neural model weights are loaded (`None`).
#[must_use]
pub fn neutral_outputs(h_prev: &[f32]) -> NeuralOutputs {
    neutral_outputs_with_actor(h_prev, None)
}

/// Deterministic neutral fallback outputs preserving both `h_prev` and optional `h_actor_prev`.
#[must_use]
pub fn neutral_outputs_with_actor(h_prev: &[f32], h_actor_prev: Option<&[f32]>) -> NeuralOutputs {
    NeuralOutputs {
        gate_logits: [0.0; 4],
        gate_probs: [0.25; 4],
        predicted_class: GateClass::Silent,
        wait_delta_ms: 0.0,
        epistemic_uncertainty: 0.0,
        cic_abstain: true,
        h_next: h_prev.to_vec(),
        h_actor_next: h_actor_prev.map(<[f32]>::to_vec).unwrap_or_default(),
        p_uncertainty: None,
        p_verify: None,
        p_encode_importance: None,
        p_memory_relevance: None,
        compute_tier_probs: None,
        compute_tier: None,
        p_user_correction: None,
        model_blake3: None,
        feature_schema_version: 0,
        is_neutral_fallback: true,
        online_adaptation_applied: false,
    }
}

/// Executes one Pure-Rust `f32` forward step of the Neural Cognitive Controller.
///
/// When `model` is `None`, returns deterministic neutral fallback outputs without
/// mutating `h_prev` or calling any neural operations.
///
/// Delegates to [`forward_step_with_actor`] with `h_actor_prev = None` (defaulting to zeros
/// of length `h_actor_dim` when a V2.2 actor-memory model is loaded).
pub fn forward_step(
    model: Option<&NeuralModel>,
    e_256: &[f32],
    x_num: &[f32],
    h_prev: &[f32],
    adapt: Option<(&PlaceAdaptState, &AdaptConfig)>,
) -> Result<NeuralOutputs, NeuralError> {
    forward_step_with_actor(model, e_256, x_num, h_prev, None, adapt)
}

/// Executes one Pure-Rust `f32` forward step of the Neural Cognitive Controller with optional
/// audience-scoped per-participant recurrent working memory `h_actor_prev`.
///
/// Backward-compatible schema slicing: if a legacy `schema = 1` (`d_in_num == 18`) model receives
/// a `32d` `schema = 2` `x_num` slice, only `&x_num[..18]` is consumed while shorter inputs are
/// still rejected loudly.
pub fn forward_step_with_actor(
    model: Option<&NeuralModel>,
    e_256: &[f32],
    x_num: &[f32],
    h_prev: &[f32],
    h_actor_prev: Option<&[f32]>,
    adapt: Option<(&PlaceAdaptState, &AdaptConfig)>,
) -> Result<NeuralOutputs, NeuralError> {
    let Some(m) = model else {
        return Ok(neutral_outputs_with_actor(h_prev, h_actor_prev));
    };

    let cfg = &m.config;
    validate_input_slice("e_256", e_256, cfg.d_in_emb)?;
    let x_num_eff = if x_num.len() == DEFAULT_NUM_FEATURES_V2 && cfg.d_in_num == DEFAULT_NUM_FEATURES_V1 {
        &x_num[..cfg.d_in_num]
    } else {
        x_num
    };
    validate_input_slice("x_num", x_num_eff, cfg.d_in_num)?;
    validate_input_slice("h_prev", h_prev, cfg.h_dim)?;

    let h_actor_owned: Vec<f32>;
    let h_actor_slice: &[f32] = if cfg.h_actor_dim == 0 {
        if let Some(a) = h_actor_prev
            && !a.is_empty()
        {
            return Err(NeuralError::InputDimensionMismatch {
                field: "h_actor_prev",
                expected: 0,
                actual: a.len(),
            });
        }
        &[]
    } else if let Some(a) = h_actor_prev {
        validate_input_slice("h_actor_prev", a, cfg.h_actor_dim)?;
        a
    } else {
        h_actor_owned = vec![0.0_f32; cfg.h_actor_dim];
        &h_actor_owned
    };

    // 1. Embedding stem: LayerNorm(W_e * e_256 + b_e)
    let mut e_proj = vec![0.0_f32; cfg.d_e];
    linear_into(
        &m.emb_linear_weight,
        Some(&m.emb_linear_bias),
        e_256,
        cfg.d_e,
        cfg.d_in_emb,
        &mut e_proj,
    );
    layer_norm_in_place(&mut e_proj, &m.emb_norm_weight, &m.emb_norm_bias);

    // 2. PLR numeric embeddings: SiLU(W_plr * [sin(2*pi*f*x), cos(2*pi*f*x)] + b_plr)
    let n_freq = cfg.plr_n_frequencies;
    let plr_in_dim = cfg.d_in_num * 2 * n_freq;
    let mut periodic = vec![0.0_f32; plr_in_dim];
    for (i, &x_val) in x_num_eff.iter().enumerate() {
        let freq_row = &m.plr_freqs[i * n_freq..(i + 1) * n_freq];
        let base = i * 2 * n_freq;
        for (j, &freq) in freq_row.iter().enumerate() {
            let angle = TAU * freq * x_val;
            let (s, c) = angle.sin_cos();
            periodic[base + j] = s;
            periodic[base + n_freq + j] = c;
        }
    }
    let mut n_proj = vec![0.0_f32; cfg.d_n];
    linear_into(
        &m.plr_linear_weight,
        Some(&m.plr_linear_bias),
        &periodic,
        cfg.d_n,
        plr_in_dim,
        &mut n_proj,
    );
    for v in &mut n_proj {
        *v = silu(*v);
    }

    // 3. Modality-Fusion concat [e_proj, n_proj, h_prev, h_actor_prev] -> d_model
    let fusion_in_dim = cfg.d_e + cfg.d_n + cfg.h_dim + cfg.h_actor_dim;
    let mut fused_in = Vec::with_capacity(fusion_in_dim);
    fused_in.extend_from_slice(&e_proj);
    fused_in.extend_from_slice(&n_proj);
    fused_in.extend_from_slice(h_prev);
    fused_in.extend_from_slice(h_actor_slice);

    let mut z0 = vec![0.0_f32; cfg.d_model];
    linear_into(
        &m.fusion_weight,
        Some(&m.fusion_bias),
        &fused_in,
        cfg.d_model,
        fusion_in_dim,
        &mut z0,
    );

    // 4. TabM k-member implicit ensemble over N Pre-LN Residual SwiGLU blocks
    let k = cfg.tabm_k;
    let d_model = cfg.d_model;
    let d_ff = cfg.d_ff;

    let mut member_states = vec![0.0_f32; k * d_model];
    for member in 0..k {
        let scale = &m.tabm_in_scale[member * d_model..(member + 1) * d_model];
        let dst = &mut member_states[member * d_model..(member + 1) * d_model];
        for i in 0..d_model {
            dst[i] = z0[i] * scale[i];
        }
    }

    let mut norm_buf = vec![0.0_f32; d_model];
    let mut gate_buf = vec![0.0_f32; d_ff];
    let mut up_buf = vec![0.0_f32; d_ff];
    let mut down_buf = vec![0.0_f32; d_model];

    for block in &m.blocks {
        for member in 0..k {
            let x_m = &mut member_states[member * d_model..(member + 1) * d_model];
            norm_buf.copy_from_slice(x_m);
            layer_norm_in_place(&mut norm_buf, &block.norm_weight, &block.norm_bias);

            let r_m = &block.tabm_r[member * d_model..(member + 1) * d_model];
            for i in 0..d_model {
                norm_buf[i] *= r_m[i];
            }

            linear_into(&block.w_gate, None, &norm_buf, d_ff, d_model, &mut gate_buf);
            linear_into(&block.w_up, None, &norm_buf, d_ff, d_model, &mut up_buf);
            for i in 0..d_ff {
                gate_buf[i] = silu(gate_buf[i]) * up_buf[i];
            }
            linear_into(&block.w_down, None, &gate_buf, d_model, d_ff, &mut down_buf);

            let s_m = &block.tabm_s[member * d_model..(member + 1) * d_model];
            for i in 0..d_model {
                x_m[i] += down_buf[i] * s_m[i];
            }
        }
    }

    // Apply final_norm and optional per-place FiLM adaptation per ensemble member,
    // and compute the ensemble-mean representation h_mean.
    let (adapt_active, logit_offsets) = match adapt {
        Some((state, adapt_cfg)) if adapt_cfg.online_adaptation => {
            (true, state.effective_logit_offsets(adapt_cfg))
        }
        _ => (false, [0.0_f32; 4]),
    };

    let mut h_mean = vec![0.0_f32; d_model];
    for member in 0..k {
        let x_m = &mut member_states[member * d_model..(member + 1) * d_model];
        layer_norm_in_place(x_m, &m.final_norm_weight, &m.final_norm_bias);
        if let Some((state, adapt_cfg)) = adapt
            && adapt_cfg.online_adaptation
        {
            state.apply_film(x_m, adapt_cfg);
        }
        for i in 0..d_model {
            h_mean[i] += x_m[i];
        }
    }
    let inv_k = 1.0 / (k as f32);
    for v in &mut h_mean {
        *v *= inv_k;
    }

    // 5. Per-place and optional per-actor recurrent working memory updates
    let h_next = step_memory_cell(&m.memory_cell, &h_mean, h_prev, cfg.h_dim, d_model);
    let h_actor_next = match &m.actor_memory_cell {
        Some(actor_cell) if cfg.h_actor_dim > 0 => {
            step_memory_cell(actor_cell, &h_mean, h_actor_slice, cfg.h_actor_dim, d_model)
        }
        _ => Vec::new(),
    };

    // 6. Phase 1 required heads: gate_4 (per-member softmax + JSD uncertainty) + wait_delta_t
    let mut gate_logits = [0.0_f32; 4];
    let mut gate_probs = [0.0_f32; 4];
    let mut mean_member_entropy = 0.0_f32;
    let mut member_logits = [0.0_f32; 4];

    for member in 0..k {
        let x_m = &member_states[member * d_model..(member + 1) * d_model];
        linear_into(
            &m.gate_4_weight,
            Some(&m.gate_4_bias),
            x_m,
            4,
            d_model,
            &mut member_logits,
        );
        for g in 0..4 {
            member_logits[g] += logit_offsets[g];
            gate_logits[g] += member_logits[g];
        }
        let probs_m = softmax_4_scaled(&member_logits, cfg.temperature);
        mean_member_entropy += entropy_4(&probs_m);
        for g in 0..4 {
            gate_probs[g] += probs_m[g];
        }
    }
    for g in 0..4 {
        gate_logits[g] *= inv_k;
        gate_probs[g] *= inv_k;
    }
    mean_member_entropy *= inv_k;

    let ensemble_entropy = entropy_4(&gate_probs);
    let epistemic_uncertainty = if k > 1 {
        ((ensemble_entropy - mean_member_entropy).max(0.0) / LN_4).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let (best_idx, max_prob) = gate_probs
        .iter()
        .copied()
        .enumerate()
        .fold((0usize, f32::NEG_INFINITY), |(bi, bp), (i, p)| {
            if p > bp { (i, p) } else { (bi, bp) }
        });
    let predicted_class = GateClass::from_index(best_idx).unwrap_or(GateClass::Silent);
    let cic_abstain = max_prob < cfg.cic_threshold;

    let wait_logit = dot(&m.wait_delta_t_weight, &h_mean) + m.wait_delta_t_bias;
    let wait_delta_ms = (sigmoid(wait_logit) * cfg.max_wait_ms).clamp(0.0, cfg.max_wait_ms);

    // 7. Optional Phase 2 & 3 heads
    let (p_uncertainty, p_verify) = match &m.phase2 {
        Some(p2) => {
            let head_unc = sigmoid(dot(&p2.p_uncertainty_weight, &h_mean) + p2.p_uncertainty_bias);
            let fused_unc = (head_unc + epistemic_uncertainty).clamp(0.0, 1.0);
            let verify = sigmoid(dot(&p2.p_verify_weight, &h_mean) + p2.p_verify_bias);
            (Some(fused_unc), Some(verify))
        }
        None => (None, None),
    };

    let (p_encode_importance, p_memory_relevance) = match &m.phase3 {
        Some(p3) => {
            let enc = sigmoid(dot(&p3.p_encode_importance_weight, &h_mean) + p3.p_encode_importance_bias);
            let rel = sigmoid(dot(&p3.p_memory_relevance_weight, &h_mean) + p3.p_memory_relevance_bias);
            (Some(enc), Some(rel))
        }
        None => (None, None),
    };

    // 8. Optional V2.2 heads: compute_tier (with ODAR escalation floor) & p_user_correction
    let (compute_tier_probs, compute_tier, p_user_correction) = match &m.v22_heads {
        Some(v22) => {
            let mut ct_logits = [0.0_f32; 3];
            linear_into(
                &v22.compute_tier_weight,
                Some(&v22.compute_tier_bias),
                &h_mean,
                3,
                d_model,
                &mut ct_logits,
            );
            let ct_probs = softmax_3(&ct_logits);
            let (ct_best_idx, _) = ct_probs
                .iter()
                .copied()
                .enumerate()
                .fold((0usize, f32::NEG_INFINITY), |(bi, bp), (i, p)| {
                    if p > bp { (i, p) } else { (bi, bp) }
                });
            let raw_tier = ComputeTier::from_index(ct_best_idx).unwrap_or(ComputeTier::Standard);
            let tier = raw_tier.escalate_with_odar(p_verify, epistemic_uncertainty);
            let p_corr =
                sigmoid(dot(&v22.p_user_correction_weight, &h_mean) + v22.p_user_correction_bias);
            (Some(ct_probs), Some(tier), Some(p_corr))
        }
        None => (None, None, None),
    };

    Ok(NeuralOutputs {
        gate_logits,
        gate_probs,
        predicted_class,
        wait_delta_ms,
        epistemic_uncertainty,
        cic_abstain,
        h_next,
        h_actor_next,
        p_uncertainty,
        p_verify,
        p_encode_importance,
        p_memory_relevance,
        compute_tier_probs,
        compute_tier,
        p_user_correction,
        model_blake3: Some(m.blake3),
        feature_schema_version: cfg.feature_schema_version,
        is_neutral_fallback: false,
        online_adaptation_applied: adapt_active,
    })
}

fn step_memory_cell(
    cell: &RecurrentCellWeights,
    h_mean: &[f32],
    h_prev: &[f32],
    h_dim: usize,
    d_model: usize,
) -> Vec<f32> {
    let mut h_next = vec![0.0_f32; h_dim];
    match cell {
        RecurrentCellWeights::MinGru {
            w_z_weight,
            w_z_bias,
            w_h_weight,
            w_h_bias,
        } => {
            let mut z = vec![0.0_f32; h_dim];
            let mut h_tilde = vec![0.0_f32; h_dim];
            linear_into(w_z_weight, Some(w_z_bias), h_mean, h_dim, d_model, &mut z);
            linear_into(w_h_weight, Some(w_h_bias), h_mean, h_dim, d_model, &mut h_tilde);
            for i in 0..h_dim {
                let z_t = sigmoid(z[i]);
                h_next[i] = (1.0 - z_t) * h_prev[i] + z_t * h_tilde[i];
            }
        }
        RecurrentCellWeights::GatedDeltaNet2 {
            w_erase_weight,
            w_erase_bias,
            w_write_weight,
            w_write_bias,
            w_delta_weight,
            w_delta_bias,
        } => {
            let mut erase = vec![0.0_f32; h_dim];
            let mut write = vec![0.0_f32; h_dim];
            let mut delta = vec![0.0_f32; h_dim];
            linear_into(w_erase_weight, Some(w_erase_bias), h_mean, h_dim, d_model, &mut erase);
            linear_into(w_write_weight, Some(w_write_bias), h_mean, h_dim, d_model, &mut write);
            linear_into(w_delta_weight, Some(w_delta_bias), h_mean, h_dim, d_model, &mut delta);
            for i in 0..h_dim {
                let b_t = sigmoid(erase[i]);
                let w_t = sigmoid(write[i]);
                let d_t = delta[i].tanh();
                h_next[i] = (1.0 - b_t) * h_prev[i] + w_t * d_t;
            }
        }
    }
    h_next
}

fn validate_input_slice(field: &'static str, slice: &[f32], expected: usize) -> Result<(), NeuralError> {
    if slice.len() != expected {
        return Err(NeuralError::InputDimensionMismatch {
            field,
            expected,
            actual: slice.len(),
        });
    }
    for (idx, &v) in slice.iter().enumerate() {
        if !v.is_finite() {
            return Err(NeuralError::NonFiniteInput { field, index: idx });
        }
    }
    Ok(())
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
}

fn linear_into(
    weight: &[f32],
    bias: Option<&[f32]>,
    input: &[f32],
    out_dim: usize,
    in_dim: usize,
    out: &mut [f32],
) {
    for row_idx in 0..out_dim {
        let row = &weight[row_idx * in_dim..(row_idx + 1) * in_dim];
        let acc = dot(row, input);
        out[row_idx] = match bias {
            Some(b) => acc + b[row_idx],
            None => acc,
        };
    }
}

fn layer_norm_in_place(x: &mut [f32], weight: &[f32], bias: &[f32]) {
    let n = x.len() as f32;
    let mean = x.iter().copied().sum::<f32>() / n;
    let var = x
        .iter()
        .map(|&v| {
            let d = v - mean;
            d * d
        })
        .sum::<f32>()
        / n;
    let inv_std = 1.0 / (var + LAYER_NORM_EPS).sqrt();
    for i in 0..x.len() {
        x[i] = ((x[i] - mean) * inv_std) * weight[i] + bias[i];
    }
}

#[inline]
fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        let z = (-x).exp();
        1.0 / (1.0 + z)
    } else {
        let z = x.exp();
        z / (1.0 + z)
    }
}

#[inline]
fn silu(x: f32) -> f32 {
    x * sigmoid(x)
}

fn softmax_3(logits: &[f32; 3]) -> [f32; 3] {
    let max_val = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut exps = [0.0_f32; 3];
    let mut sum = 0.0_f32;
    for i in 0..3 {
        exps[i] = (logits[i] - max_val).exp();
        sum += exps[i];
    }
    let inv_sum = 1.0 / sum;
    [exps[0] * inv_sum, exps[1] * inv_sum, exps[2] * inv_sum]
}

fn softmax_4_scaled(logits: &[f32; 4], temperature: f32) -> [f32; 4] {
    let inv_t = 1.0 / temperature;
    let scaled = [
        logits[0] * inv_t,
        logits[1] * inv_t,
        logits[2] * inv_t,
        logits[3] * inv_t,
    ];
    let max_val = scaled.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut exps = [0.0_f32; 4];
    let mut sum = 0.0_f32;
    for i in 0..4 {
        exps[i] = (scaled[i] - max_val).exp();
        sum += exps[i];
    }
    let inv_sum = 1.0 / sum;
    [
        exps[0] * inv_sum,
        exps[1] * inv_sum,
        exps[2] * inv_sum,
        exps[3] * inv_sum,
    ]
}

fn entropy_4(probs: &[f32; 4]) -> f32 {
    probs
        .iter()
        .copied()
        .filter(|&p| p > 0.0)
        .map(|p| -p * p.ln())
        .sum()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::neural::CellType;
    use crate::neural::weights::tests_support::build_synthetic_safetensors;

    fn assert_outputs_valid(out: &NeuralOutputs, h_dim: usize, max_wait_ms: f32) {
        assert!(!out.is_neutral_fallback);
        assert_eq!(out.h_next.len(), h_dim);
        for &v in &out.gate_logits {
            assert!(v.is_finite());
        }
        let prob_sum: f32 = out.gate_probs.iter().sum();
        assert!((prob_sum - 1.0).abs() < 1e-5, "prob_sum={prob_sum}");
        for &p in &out.gate_probs {
            assert!(p.is_finite() && (0.0..=1.0).contains(&p));
        }
        assert!(out.wait_delta_ms.is_finite() && out.wait_delta_ms >= 0.0 && out.wait_delta_ms <= max_wait_ms);
        assert!(
            out.epistemic_uncertainty.is_finite()
                && (0.0..=1.0).contains(&out.epistemic_uncertainty)
        );
        for &h in &out.h_next {
            assert!(h.is_finite());
        }
    }

    #[test]
    fn forward_150k_and_1m_models_with_both_recurrent_cells() {
        let configs = [
            // ~150k param regime, minGRU, k=8
            (64, 32, 128, 192, 2, 8, 8, CellType::MinGru, false, false),
            // ~1M param regime, Gated DeltaNet-2, k=8, with Phase 2+3 heads
            (128, 64, 256, 640, 2, 8, 16, CellType::GatedDeltaNet2, true, true),
        ];

        let e_256 = vec![0.15_f32; 256];
        let x_num = vec![0.4_f32; 18];

        for (d_e, d_n, d_model, d_ff, n_layers, k, n_freq, cell, p2, p3) in configs {
            let bytes = build_synthetic_safetensors(
                256, 18, d_e, d_n, d_model, d_ff, n_layers, k, 64, n_freq, cell, p2, p3,
            );
            let model = NeuralModel::from_safetensors_bytes(&bytes, None).expect("valid model");
            let mut h_prev = vec![0.0_f32; 64];

            let t0 = Instant::now();
            let out = forward_step(Some(&model), &e_256, &x_num, &h_prev, None).expect("forward");
            let elapsed = t0.elapsed();

            assert_outputs_valid(&out, 64, model.config.max_wait_ms);
            assert_eq!(out.p_uncertainty.is_some(), p2);
            assert_eq!(out.p_verify.is_some(), p2);
            assert_eq!(out.p_encode_importance.is_some(), p3);
            assert_eq!(out.p_memory_relevance.is_some(), p3);
            assert!(
                elapsed.as_millis() < 200,
                "single forward pass should stay fast, took {elapsed:?}"
            );

            // Second recurrent step verifies working memory evolves deterministically.
            h_prev = out.h_next.clone();
            let out2 = forward_step(Some(&model), &e_256, &x_num, &h_prev, None).expect("step 2");
            assert_outputs_valid(&out2, 64, model.config.max_wait_ms);
            assert_ne!(out.h_next, out2.h_next);
        }
    }

    #[test]
    fn tabm_k1_has_zero_epistemic_spread_and_k8_exposes_nonnegative_spread() {
        let e_256 = vec![0.25_f32; 256];
        let x_num = vec![0.6_f32; 18];
        let h_prev = vec![0.0_f32; 64];

        let bytes_k1 = build_synthetic_safetensors(
            256, 18, 64, 32, 128, 192, 2, 1, 64, 8, CellType::MinGru, false, false,
        );
        let model_k1 = NeuralModel::from_safetensors_bytes(&bytes_k1, None).expect("k=1");
        let out_k1 = forward_step(Some(&model_k1), &e_256, &x_num, &h_prev, None).expect("k=1 fwd");
        assert_eq!(out_k1.epistemic_uncertainty, 0.0);

        let bytes_k8 = build_synthetic_safetensors(
            256, 18, 64, 32, 128, 192, 2, 8, 64, 8, CellType::MinGru, false, false,
        );
        let model_k8 = NeuralModel::from_safetensors_bytes(&bytes_k8, None).expect("k=8");
        let out_k8 = forward_step(Some(&model_k8), &e_256, &x_num, &h_prev, None).expect("k=8 fwd");
        assert!(out_k8.epistemic_uncertainty > 0.0);
        assert!(out_k8.epistemic_uncertainty <= 1.0);
    }

    #[test]
    fn vectorless_zero_embedding_and_neutral_fallback_run_cleanly() {
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
            CellType::GatedDeltaNet2,
            false,
            false,
        );
        let model = NeuralModel::from_safetensors_bytes(&bytes, None).expect("valid");
        let zero_emb = vec![0.0_f32; 256];
        let x_num = vec![0.2_f32; 18];
        let h_prev = vec![0.1_f32; 64];

        let out = forward_step(Some(&model), &zero_emb, &x_num, &h_prev, None).expect("vectorless");
        assert_outputs_valid(&out, 64, model.config.max_wait_ms);
        assert!(out.p_uncertainty.is_none());
        assert!(out.p_verify.is_none());
        assert!(out.p_encode_importance.is_none());
        assert!(out.p_memory_relevance.is_none());

        // Neutral fallback when model is None
        let neutral = forward_step(None, &zero_emb, &x_num, &h_prev, None).expect("neutral");
        assert!(neutral.is_neutral_fallback);
        assert_eq!(neutral.gate_probs, [0.25; 4]);
        assert_eq!(neutral.h_next, h_prev);
        assert_eq!(neutral.model_blake3, None);
        assert_eq!(neutral.feature_schema_version, 0);
        assert!(neutral.p_uncertainty.is_none());
        assert!(neutral.p_memory_relevance.is_none());
    }

    #[test]
    fn forward_step_latency_across_model_sizes() {
        let presets = [
            ("150k", 64, 32, 96, 192, 2, 8, 64, 8),
            ("500k", 96, 48, 160, 384, 2, 8, 64, 8),
            ("1M", 128, 64, 224, 512, 3, 8, 64, 8),
            ("3M", 128, 64, 320, 768, 4, 8, 64, 8),
        ];
        let e_256 = vec![0.12_f32; 256];
        let x_num = vec![0.35_f32; 18];
        let iters: u32 = if cfg!(debug_assertions) { 5 } else { 100 };

        for (label, d_e, d_n, d_model, d_ff, n_layers, k, h_dim, n_freq) in presets {
            for cell in [CellType::MinGru, CellType::GatedDeltaNet2] {
                let bytes = build_synthetic_safetensors(
                    256, 18, d_e, d_n, d_model, d_ff, n_layers, k, h_dim, n_freq, cell, false,
                    false,
                );
                let model = NeuralModel::from_safetensors_bytes(&bytes, None).expect("model");
                let mut h_prev = vec![0.0_f32; h_dim];

                for _ in 0..3 {
                    let out =
                        forward_step(Some(&model), &e_256, &x_num, &h_prev, None).expect("warmup");
                    h_prev = out.h_next;
                }

                let t0 = Instant::now();
                let mut last_prob = 0.0_f32;
                for _ in 0..iters {
                    let out =
                        forward_step(Some(&model), &e_256, &x_num, &h_prev, None).expect("bench");
                    h_prev = out.h_next;
                    last_prob = out.gate_probs[3];
                }
                let us_per_step = t0.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(iters);
                assert!(last_prob.is_finite());
                eprintln!(
                    "[NEURAL_LATENCY] size={label} cell={cell:?} us_per_step={us_per_step:.2} iters={iters}"
                );
            }
        }
    }

    #[test]
    fn forward_v22_with_h_actor_v22_heads_and_odar_escalation() {
        let e_256 = vec![0.15_f32; 256];
        let x_32 = vec![0.35_f32; 32];
        for cell in [CellType::MinGru, CellType::GatedDeltaNet2] {
            let bytes = crate::neural::weights::tests_support::build_synthetic_v22_safetensors(
                2, 256, 32, 64, 32, 128, 192, 2, 4, 64, 24, 8, cell, true, true, true,
            );
            let model = NeuralModel::from_safetensors_bytes(&bytes, None).expect("v22 model");
            let h_prev = vec![0.0_f32; 64];

            // forward_step delegates with zero-initialized h_actor_prev.
            let out1 = forward_step(Some(&model), &e_256, &x_32, &h_prev, None).expect("step1");
            assert_outputs_valid(&out1, 64, model.config.max_wait_ms);
            assert_eq!(out1.h_actor_next.len(), 24);
            assert!(out1.h_actor_next.iter().all(|v| v.is_finite()));
            let ct_probs = out1.compute_tier_probs.expect("compute_tier_probs");
            let sum: f32 = ct_probs.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
            assert!(out1.compute_tier.is_some());
            let p_corr = out1.p_user_correction.expect("p_user_correction");
            assert!((0.0..=1.0).contains(&p_corr));

            // Stepping with evolved h_actor_prev produces distinct recurrent actor state.
            let out2 = forward_step_with_actor(
                Some(&model),
                &e_256,
                &x_32,
                &out1.h_next,
                Some(&out1.h_actor_next),
                None,
            )
            .expect("step2");
            assert_ne!(out1.h_actor_next, out2.h_actor_next);
        }

        // Verify ODAR epistemic escalation floor rules.
        assert_eq!(ComputeTier::Fast.escalate_with_odar(Some(0.20), 0.10), ComputeTier::Fast);
        assert_eq!(ComputeTier::Fast.escalate_with_odar(Some(0.70), 0.10), ComputeTier::Standard);
        assert_eq!(ComputeTier::Fast.escalate_with_odar(Some(0.20), 0.35), ComputeTier::Standard);
        assert_eq!(ComputeTier::Fast.escalate_with_odar(Some(0.85), 0.10), ComputeTier::Deep);
        assert_eq!(ComputeTier::Standard.escalate_with_odar(Some(0.10), 0.55), ComputeTier::Deep);
        assert_eq!(ComputeTier::Deep.escalate_with_odar(None, 0.0), ComputeTier::Deep);
    }

    #[test]
    fn schema_v1_model_slices_32d_x_num_cleanly_and_rejects_short_inputs() {
        let bytes = build_synthetic_safetensors(
            256, 18, 64, 32, 128, 192, 2, 4, 64, 8, CellType::MinGru, true, true,
        );
        let model = NeuralModel::from_safetensors_bytes(&bytes, None).expect("v1 model");
        let e_256 = vec![0.1_f32; 256];
        let mut x_32 = vec![0.25_f32; 32];
        for (i, v) in x_32.iter_mut().enumerate() {
            *v = ((i as f32) + 1.0) / 33.0;
        }
        let h_prev = vec![0.0_f32; 64];

        let out_18 =
            forward_step(Some(&model), &e_256, &x_32[..18], &h_prev, None).expect("18d");
        let out_32 = forward_step(Some(&model), &e_256, &x_32, &h_prev, None).expect("32d sliced");
        assert_eq!(out_18, out_32);
        assert!(out_32.h_actor_next.is_empty());
        assert!(out_32.compute_tier.is_none());
        assert!(out_32.p_user_correction.is_none());

        // Short input (17d) and invalid intermediate length (20d) must still be rejected.
        let err_short =
            forward_step(Some(&model), &e_256, &x_32[..17], &h_prev, None).expect_err("too short");
        assert_eq!(
            err_short,
            NeuralError::InputDimensionMismatch {
                field: "x_num",
                expected: 18,
                actual: 17,
            }
        );
        let err_mid =
            forward_step(Some(&model), &e_256, &x_32[..20], &h_prev, None).expect_err("20d bad");
        assert_eq!(
            err_mid,
            NeuralError::InputDimensionMismatch {
                field: "x_num",
                expected: 18,
                actual: 20,
            }
        );
    }
}
