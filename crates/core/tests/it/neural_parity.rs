use std::fs;
use std::path::PathBuf;

use monoagens_core::blake3;
use monoagens_core::neural::{NeuralModel, forward_step};
use serde::Deserialize;

#[derive(Deserialize)]
struct GoldenFixture {
    cases: Vec<GoldenCase>,
}

#[derive(Deserialize)]
struct GoldenCase {
    name: String,
    safetensors_file: String,
    blake3: String,
    steps: Vec<GoldenStep>,
}

#[derive(Deserialize)]
struct GoldenStep {
    e_256: Vec<f32>,
    x_num: Vec<f32>,
    expected_gate_logits: [f32; 4],
    expected_gate_probs: [f32; 4],
    expected_wait_delta_ms: f32,
    expected_epistemic_uncertainty: f32,
    expected_cic_abstain: bool,
    expected_h_next: Vec<f32>,
    #[serde(default)]
    expected_p_uncertainty: Option<f32>,
    #[serde(default)]
    expected_p_verify: Option<f32>,
    #[serde(default)]
    expected_p_encode_importance: Option<f32>,
    #[serde(default)]
    expected_p_memory_relevance: Option<f32>,
}

fn assert_opt_f32_close(label: &str, actual: Option<f32>, expected: Option<f32>, tol: f32) -> f32 {
    match (actual, expected) {
        (Some(a), Some(e)) => {
            let d = (a - e).abs();
            assert!(d <= tol, "{label} diff {d} > {tol} (rust={a}, py={e})");
            d
        }
        (None, None) => 0.0,
        _ => panic!("{label} presence mismatch: rust={actual:?}, py={expected:?}"),
    }
}

fn verify_golden_fixture(filename: &str, expect_phase2: bool, expect_phase3: bool) {
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let json_path = fixtures_dir.join(filename);
    let json_text = fs::read_to_string(&json_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", json_path.display()));
    let fixture: GoldenFixture = serde_json::from_str(&json_text).expect("parse golden json");
    assert!(
        fixture.cases.len() >= 2,
        "expected both minGRU and Gated DeltaNet-2 parity cases"
    );

    const TOL: f32 = 1e-5;
    let mut max_abs_diff = 0.0_f32;
    let mut max_wait_diff_ms = 0.0_f32;

    for case in &fixture.cases {
        let st_path = fixtures_dir.join(&case.safetensors_file);
        let bytes = fs::read(&st_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", st_path.display()));
        let actual_hex = blake3::to_hex(&blake3::hash(&bytes));
        assert_eq!(
            actual_hex, case.blake3,
            "BLAKE3 mismatch for case {}",
            case.name
        );

        let model = NeuralModel::from_safetensors_bytes(&bytes, Some(blake3::hash(&bytes)))
            .expect("load safetensors parity model");
        assert_eq!(model.phase2.is_some(), expect_phase2);
        assert_eq!(model.phase3.is_some(), expect_phase3);
        let mut h_prev = vec![0.0_f32; model.config.h_dim];

        for (step_idx, step) in case.steps.iter().enumerate() {
            let out = forward_step(Some(&model), &step.e_256, &step.x_num, &h_prev, None)
                .expect("rust forward_step");

            for g in 0..4 {
                let d_logit = (out.gate_logits[g] - step.expected_gate_logits[g]).abs();
                max_abs_diff = max_abs_diff.max(d_logit);
                assert!(
                    d_logit <= TOL,
                    "{} step {step_idx} logit[{g}] diff {d_logit} > {TOL} (rust={}, py={})",
                    case.name,
                    out.gate_logits[g],
                    step.expected_gate_logits[g]
                );

                let d_prob = (out.gate_probs[g] - step.expected_gate_probs[g]).abs();
                max_abs_diff = max_abs_diff.max(d_prob);
                assert!(
                    d_prob <= TOL,
                    "{} step {step_idx} prob[{g}] diff {d_prob} > {TOL} (rust={}, py={})",
                    case.name,
                    out.gate_probs[g],
                    step.expected_gate_probs[g]
                );
            }

            let d_wait = (out.wait_delta_ms - step.expected_wait_delta_ms).abs();
            max_wait_diff_ms = max_wait_diff_ms.max(d_wait);
            assert!(
                d_wait <= 1e-2 && (d_wait / model.config.max_wait_ms) <= TOL,
                "{} step {step_idx} wait_delta_ms diff {d_wait} exceeded tolerance (rust={}, py={})",
                case.name,
                out.wait_delta_ms,
                step.expected_wait_delta_ms
            );

            let d_ep = (out.epistemic_uncertainty - step.expected_epistemic_uncertainty).abs();
            max_abs_diff = max_abs_diff.max(d_ep);
            assert!(
                d_ep <= TOL,
                "{} step {step_idx} epistemic diff {d_ep} > {TOL}",
                case.name
            );
            assert_eq!(
                out.cic_abstain, step.expected_cic_abstain,
                "{} step {step_idx} cic_abstain mismatch",
                case.name
            );

            max_abs_diff = max_abs_diff.max(assert_opt_f32_close(
                &format!("{} step {step_idx} p_uncertainty", case.name),
                out.p_uncertainty,
                if expect_phase2 { step.expected_p_uncertainty } else { None },
                TOL,
            ));
            max_abs_diff = max_abs_diff.max(assert_opt_f32_close(
                &format!("{} step {step_idx} p_verify", case.name),
                out.p_verify,
                if expect_phase2 { step.expected_p_verify } else { None },
                TOL,
            ));
            max_abs_diff = max_abs_diff.max(assert_opt_f32_close(
                &format!("{} step {step_idx} p_encode_importance", case.name),
                out.p_encode_importance,
                if expect_phase3 { step.expected_p_encode_importance } else { None },
                TOL,
            ));
            max_abs_diff = max_abs_diff.max(assert_opt_f32_close(
                &format!("{} step {step_idx} p_memory_relevance", case.name),
                out.p_memory_relevance,
                if expect_phase3 { step.expected_p_memory_relevance } else { None },
                TOL,
            ));

            assert_eq!(out.h_next.len(), step.expected_h_next.len());
            for (i, (&r_h, &p_h)) in out.h_next.iter().zip(&step.expected_h_next).enumerate() {
                let d_h = (r_h - p_h).abs();
                max_abs_diff = max_abs_diff.max(d_h);
                assert!(
                    d_h <= TOL,
                    "{} step {step_idx} h_next[{i}] diff {d_h} > {TOL} (rust={r_h}, py={p_h})",
                    case.name
                );
            }

            h_prev = out.h_next;
        }
    }

    assert!(max_abs_diff.is_finite());
    eprintln!(
        "{filename} parity max_abs_diff={max_abs_diff:.3e}, max_wait_diff_ms={max_wait_diff_ms:.3e}"
    );
}

#[test]
fn python_rust_neural_forward_parity_within_1e_5() {
    verify_golden_fixture("neural_phase1_golden.json", false, false);
}

#[test]
fn python_rust_phase2_neural_forward_parity_within_1e_5() {
    verify_golden_fixture("neural_phase2_golden.json", true, false);
}

#[test]
fn python_rust_phase3_neural_forward_parity_within_1e_5() {
    verify_golden_fixture("neural_phase3_golden.json", true, true);
}
