//! Versioned disposition prior with documented range validation.

use core::fmt;

use serde::{Deserialize, Serialize};

/// Errors produced when validating a [`Prior`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorError {
    UnsupportedFormatVersion {
        found: u32,
        supported: u32,
    },
    EmptyContentVersion,
    OutOfRange {
        disposition: &'static str,
        value: String,
        range: &'static str,
    },
}

impl fmt::Display for PriorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormatVersion { found, supported } => {
                write!(
                    f,
                    "unsupported prior format_version {found} (supported {supported})"
                )
            }
            Self::EmptyContentVersion => f.write_str("prior content_version must be non-empty"),
            Self::OutOfRange {
                disposition,
                value,
                range,
            } => {
                write!(
                    f,
                    "prior disposition '{disposition}' value {value} is outside documented range {range}"
                )
            }
        }
    }
}

impl std::error::Error for PriorError {}

/// Current supported format version for [`Prior`].
pub const CURRENT_PRIOR_FORMAT_VERSION: u32 = 1;

const fn default_own_name_weight() -> f64 {
    0.6
}
const fn default_info_scale() -> f64 {
    0.6
}
const fn default_fok_scale() -> f64 {
    0.4
}
const fn default_relevance_weight() -> f64 {
    0.4
}
const fn default_commitment_weight() -> f64 {
    0.2
}
const fn default_restraint_scale() -> f64 {
    0.8
}
const fn default_asr_conf_threshold() -> f64 {
    0.45
}
const fn default_asr_clarification_boost() -> f64 {
    0.35
}
const fn default_w_b() -> f64 {
    1.0
}
const fn default_context_weight() -> f64 {
    1.0
}
const fn default_importance_weight() -> f64 {
    0.5
}
const fn default_fok_threshold() -> f64 {
    0.8
}
const fn default_fok_scale_norm() -> f64 {
    0.5
}
const fn default_fade_threshold() -> f64 {
    -3.29
}
const fn default_kappa() -> f64 {
    1.0
}
const fn default_a_scale() -> f64 {
    0.6
}
const fn default_budget_weight() -> f64 {
    0.3
}
const fn default_promise_debt_weight() -> f64 {
    0.2
}
const fn default_reserve_when_unfamiliar() -> f64 {
    0.15
}
const fn default_note_band() -> f64 {
    0.1
}
const fn default_talk_rate_window_minutes() -> f64 {
    30.0
}
const fn default_talk_rate_prior_count() -> f64 {
    10.0
}
const fn default_closeness_prior_count() -> f64 {
    5.0
}
const fn default_promise_debt_horizon_ms() -> u64 {
    3_600_000
}
const fn default_lambda_c() -> f64 {
    0.35
}
const fn default_lambda_a() -> f64 {
    0.0
}
const fn default_theta_alloc() -> f64 {
    0.68
}
const fn default_theta_ex() -> f64 {
    0.50
}
const fn default_m_ex() -> f64 {
    3.0
}
const fn default_w_c() -> f64 {
    0.65
}
const fn default_w_o() -> f64 {
    0.80
}
const fn default_gamma_c() -> f64 {
    0.65
}
const fn default_tau_c_s() -> f64 {
    120.0
}
const fn default_w_c_window() -> usize {
    12
}
const fn default_k_c() -> f64 {
    0.35
}
const fn default_beta_fan() -> f64 {
    0.50
}
const fn default_theta_trace() -> f64 {
    0.45
}
const fn default_patience_place_ms() -> u64 {
    30_000
}
const fn default_backlog_max_chunks() -> usize {
    4
}
const fn default_step_init() -> f64 {
    0.10
}
const fn default_gamma_init() -> f64 {
    0.50
}
const fn default_delta_emit() -> f64 {
    1.0
}
const fn default_delta_ign() -> f64 {
    1.5
}
const fn default_c_base() -> f64 {
    0.45
}
const fn default_beta_bp() -> f64 {
    0.70
}
const fn default_beta_load() -> f64 {
    0.25
}
const fn default_kappa_bp() -> f64 {
    1.8
}
const fn default_tau_win_s() -> f64 {
    30.0
}
const fn default_lambda_sleep_init() -> f64 {
    0.20
}
const fn default_n_min_sleep() -> usize {
    32
}
const fn default_alpha_u() -> f64 {
    0.85
}
const fn default_tau_wake_hours() -> f64 {
    24.0
}
const fn default_d_sleep_max() -> f64 {
    2.0
}
const fn default_tau_sw() -> f64 {
    0.50
}
const fn default_w_fail() -> f64 {
    2.0
}
const fn default_lambda_stakes() -> f64 {
    0.50
}
const fn default_theta_sw() -> f64 {
    0.70
}
const fn default_w_sw() -> f64 {
    0.50
}
const fn default_lambda_fok() -> f64 {
    0.40
}
const fn default_lambda_surp() -> f64 {
    0.30
}
const fn default_lambda_voi() -> f64 {
    0.40
}
const fn default_c0() -> f64 {
    0.25
}
const fn default_gamma_opp() -> f64 {
    0.75
}
const fn default_alpha_pow() -> f64 {
    1.6
}
const fn default_tau_k() -> f64 {
    3.0
}
const fn default_w_rec() -> f64 {
    0.50
}
const fn default_eta_sl() -> f64 {
    0.30
}
const fn default_eta_tr() -> f64 {
    0.50
}
const fn default_q_ref() -> f64 {
    32.0
}
const fn default_b_steps() -> usize {
    8
}
const fn default_n_rec_max() -> usize {
    4
}
const fn default_patience_t_min_ms() -> u64 {
    500
}
const fn default_patience_t_max_ms() -> u64 {
    60_000
}
const fn default_patience_default_ms() -> u64 {
    10_000
}
const fn default_patience_sigma_ln() -> f64 {
    1.0
}
const fn default_patience_prior_n0() -> f64 {
    10.0
}
const fn default_patience_place_n0() -> f64 {
    5.0
}
const fn default_patience_quantile() -> f64 {
    0.10
}
const fn default_patience_decay_rho() -> f64 {
    0.995
}
const fn default_map_lambda_decay() -> f64 {
    0.000_962_7
}
const fn default_map_eta_co() -> f64 {
    1.0
}
const fn default_map_w_max() -> f64 {
    50.0
}
const fn default_map_gamma_sr() -> f64 {
    0.85
}
const fn default_map_alpha_sr() -> f64 {
    0.10
}
const fn default_map_tau_step_s() -> f64 {
    60.0
}
const fn default_map_ep_gap_s() -> f64 {
    1_800.0
}
const fn default_map_beta_sr() -> f64 {
    0.50
}
const fn default_map_delta_ppr() -> f64 {
    0.50
}
const fn default_map_hop_budget() -> usize {
    3
}
const fn default_map_k_sr() -> usize {
    32
}
const fn default_map_w_map() -> f64 {
    0.25
}
const fn default_recall_vector_top_k() -> usize {
    40
}
const fn default_recall_text_top_k() -> usize {
    20
}
const fn default_promise_retry_max() -> u32 {
    3
}
const fn default_promise_retry_base_ms() -> u64 {
    60_000
}
const fn default_agency_tolerance_k() -> f64 {
    2.0
}
const fn default_response_expiry_quantile() -> f64 {
    0.90
}
const fn default_response_stakes_factor() -> f64 {
    2.0
}
const fn default_content_miss_z_threshold() -> f64 {
    2.0
}
const fn default_content_miss_min_samples() -> usize {
    3
}
const fn default_silence_sample_rate() -> f64 {
    0.02
}
const fn default_wake_gate_lo() -> f64 {
    0.35
}
const fn default_wake_gate_hi_epsilon() -> f64 {
    0.06
}
const fn default_wake_gate_margin() -> f64 {
    0.05
}
const fn default_default_goal_expiry_ms() -> u64 {
    86_400_000
}
const fn default_pending_approval_expiry_ms() -> u64 {
    3_600_000
}
const fn default_context_budget_bytes() -> usize {
    32_768
}
const fn default_working_set_full_obs_limit() -> usize {
    8
}
const fn default_place_tail_limit() -> usize {
    16
}
const fn default_anchor_check_interval_wakes() -> u64 {
    8
}
const fn default_auto_recall_max() -> u8 {
    2
}
const fn default_tool_rag_enabled() -> bool {
    true
}
const fn default_tool_rag_k_cap() -> usize {
    8
}
const fn default_tool_rag_k_proc() -> usize {
    3
}
const fn default_tool_rag_max_static_core() -> usize {
    4
}
// --- C6 Beliefs, Credence, Corroboration, Conventions, Sleep, EM-LLM, Merge, Domains ---
const fn default_credence_lambda_domain() -> f64 {
    0.9
}
const fn default_credence_period_ms() -> u64 {
    86_400_000
}
const fn default_k_corrob() -> usize {
    2
}
const fn default_rule_trust_threshold() -> f64 {
    0.80
}
const fn default_conv_alpha_0() -> f64 {
    1.0
}
const fn default_conv_beta_0() -> f64 {
    1.0
}
const fn default_conv_lambda() -> f64 {
    0.95
}
const fn default_conv_vicarious_discount() -> f64 {
    0.30
}
const fn default_conv_peer_floor() -> f64 {
    0.10
}
const fn default_conv_tau() -> f64 {
    5.0
}
const fn default_conv_kappa() -> f64 {
    0.25
}
const fn default_conv_bias_max() -> f64 {
    0.50
}
const fn default_t_idle_sleep_ms() -> u64 {
    600_000
}
const fn default_provisional_close() -> bool {
    true
}
const fn default_n_ep_min() -> usize {
    4
}
const fn default_n_ep_max() -> usize {
    128
}
const fn default_k_stale_sleep() -> f64 {
    6.0
}
const fn default_em_boundary_gamma() -> f64 {
    1.0
}
const fn default_em_boundary_window() -> usize {
    50
}
const fn default_em_boundary_warmup() -> usize {
    8
}
const fn default_em_boundary_mu0() -> f64 {
    0.25
}
const fn default_em_boundary_sigma0() -> f64 {
    0.15
}
const fn default_em_boundary_m0() -> f64 {
    5.0
}
const fn default_em_boundary_z_max() -> f64 {
    3.0
}
const fn default_em_boundary_sigma_u_floor() -> f64 {
    0.35
}
const fn default_em_fixed_interval() -> usize {
    16
}
const fn default_merge_jaccard_min() -> f64 {
    0.80
}
const fn default_merge_gap_ms() -> u64 {
    1_800_000
}
const fn default_domain_clusters_k() -> usize {
    16
}
const fn default_domain_m_min() -> usize {
    5
}
const fn default_domain_max_iter() -> usize {
    25
}
const fn default_domain_w_hist() -> f64 {
    5.0
}
const fn default_domain_tau_match() -> f64 {
    0.70
}
const fn default_domain_tau_rel() -> f64 {
    0.50
}
const fn default_domain_tau_merge() -> f64 {
    0.75
}
const fn default_domain_gamma_drift() -> f64 {
    1.0
}
const fn default_domain_rho_split() -> f64 {
    0.0
}
// --- C7 Lessons, Procedures, Ledger, SelfCard & Bounded Learning ---
const fn default_lesson_min_episodes() -> usize {
    2
}
const fn default_lesson_max_contrasts() -> usize {
    2
}
const fn default_proc_gamma_beta() -> f64 {
    0.98
}
const fn default_proc_cusum_n_min_lock() -> usize {
    5
}
const fn default_proc_cusum_k_drift() -> f64 {
    0.15
}
const fn default_proc_cusum_h_warn() -> f64 {
    2.0
}
const fn default_proc_cusum_h_retire() -> f64 {
    3.5
}
const fn default_proc_theta_retire() -> f64 {
    0.30
}
const fn default_allow_chat_habit_bypass() -> bool {
    false
}
const fn default_ledger_max_items_per_card() -> usize {
    32
}
const fn default_self_card_cap_z() -> f64 {
    1.645
}
const fn default_self_card_cap_min_trials() -> usize {
    3
}
const fn default_self_card_cap_deg_threshold() -> f64 {
    0.40
}
const fn default_self_card_cap_consec_fail() -> u32 {
    3
}
const fn default_self_card_kappa_bias() -> f64 {
    1.5
}
const fn default_self_card_cal_n0() -> f64 {
    5.0
}
const fn default_learn_rate_w() -> f64 {
    0.02
}
const fn default_learn_rate_theta_ratio() -> f64 {
    0.20
}
const fn default_learn_daily_max_delta() -> f64 {
    0.05
}
const fn default_learn_ips_clip_m() -> f64 {
    10.0
}
const fn default_learn_wake_gate_n0() -> f64 {
    5.0
}
const fn default_learn_wake_gate_quantile() -> f64 {
    0.10
}

/// Dispositions and prior bounds configuring Core's appraisal, memory, and learning mechanisms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prior {
    /// Schema format version of the prior configuration.
    pub version: u32,
    /// Content identifier for the prior calibration set (e.g. `"2026-11-16.1"`).
    pub content_version: String,
    /// Maps baseline wake threshold `theta_0` (`[0.0, 1.0]`).
    pub talkativeness: f64,
    /// Maps stochastic go/no-go temperature `tau` (`[0.0, 1.0]`).
    pub spontaneity: f64,
    /// Maps ACT-R/Petrov decay exponent `d` (`[0.0, 1.0]`).
    pub memory_persistence: f64,
    /// Scales positive salience weights (`[0.0, 1.0]`).
    pub helpfulness: f64,
    /// Scales social cost and talk-rate restraint (`[0.0, 1.0]`).
    pub restraint: f64,
    /// Maps habituation decay rate `gamma` (`[0.0, 1.0]`).
    pub boredom_with_repeats: f64,
    /// Minimum certainty bar for report control `P_rc` (`[0.0, 1.0]`).
    pub certainty_bar: f64,
    /// Motivation speaking bar on `[1, 5]` (`1..=5`).
    pub speaking_bar: u8,

    // --- Salience & signal weights (R-03, A-01, A-14, R-37) ---
    /// Weight on `own_name` signal (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_own_name_weight")]
    pub own_name_weight: f64,
    /// Scale factor for `w_info = info_scale * helpfulness` (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_info_scale")]
    pub info_scale: f64,
    /// Scale factor for `w_fok = fok_scale * helpfulness` (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_fok_scale")]
    pub fok_scale: f64,
    /// Weight on `relevance` signal (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_relevance_weight")]
    pub relevance_weight: f64,
    /// Weight on unallocated `commitment` / `deg(e, soul)` signal (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_commitment_weight")]
    pub commitment_weight: f64,
    /// Scale factor for `w_sc = -restraint_scale * restraint` (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_restraint_scale")]
    pub restraint_scale: f64,
    /// Speech ASR confidence cutoff below which clarification boost fires (`[0.0, 1.0]`).
    #[serde(default = "default_asr_conf_threshold")]
    pub asr_conf_threshold: f64,
    /// Additive boost to `info_gap` when `asr_conf < asr_conf_threshold` (`[0.0, 1.0]`).
    #[serde(default = "default_asr_clarification_boost")]
    pub asr_clarification_boost: f64,

    // --- FOK & Recall activation weights (R-03, A-01, A-07) ---
    /// Row-10 base-level activation weight `w_B` (`[0.0, 2.0]`, neutral `0.0`).
    #[serde(default = "default_w_b")]
    pub w_b: f64,
    /// Cosine similarity weight in `A` (`[0.0, 3.0]`).
    #[serde(default = "default_context_weight")]
    pub context_weight: f64,
    /// Importance weight in `A` (`[0.0, 2.0]`).
    #[serde(default = "default_importance_weight")]
    pub importance_weight: f64,
    /// Logistic midpoint `fok_threshold` for `sigma((A* - fok_threshold) / fok_scale_norm)` (`[-5.0, 5.0]`).
    #[serde(default = "default_fok_threshold")]
    pub fok_threshold: f64,
    /// Logistic scale `fok_scale_norm` (`[0.05, 5.0]`).
    #[serde(default = "default_fok_scale_norm")]
    pub fok_scale_norm: f64,
    /// Petrov base-level fade threshold (`[-10.0, 0.0]`).
    #[serde(default = "default_fade_threshold")]
    pub fade_threshold: f64,

    // --- Habituation & Threshold drives (R-01, R-03, A-01, A-03, A-07) ---
    /// Habituation gain `kappa` in `S / (1 + kappa * h_pre)` (`[0.0, 2.0]`, neutral `0.0`).
    #[serde(default = "default_kappa")]
    pub kappa: f64,
    /// Talk-rate drive multiplier scale `a = a_scale * restraint` (`[0.0, 1.5]`, neutral `0.0`).
    #[serde(default = "default_a_scale")]
    pub a_scale: f64,
    /// Daily budget drive weight `b` (`[0.0, 0.6]`, neutral `0.0`).
    #[serde(default = "default_budget_weight")]
    pub budget_weight: f64,
    /// Promise debt drive weight `c` (`[0.0, 0.5]`, neutral `0.0`).
    #[serde(default = "default_promise_debt_weight")]
    pub promise_debt_weight: f64,
    /// Unfamiliarity reserve weight `r` (`[0.0, 0.3]`, neutral `0.0`).
    #[serde(default = "default_reserve_when_unfamiliar")]
    pub reserve_when_unfamiliar: f64,
    /// Minimum probability band to record `Note` instead of `Silent` (`[0.01, 0.3]`).
    #[serde(default = "default_note_band")]
    pub note_band: f64,
    /// Rolling window for `talk_rate` in minutes (`[5.0, 120.0]`).
    #[serde(default = "default_talk_rate_window_minutes")]
    pub talk_rate_window_minutes: f64,
    /// `A-07` pseudo-count shrinkage weight for `talk_rate` (`[0.0, 100.0]`).
    #[serde(default = "default_talk_rate_prior_count")]
    pub talk_rate_prior_count: f64,
    /// `A-07` pseudo-count shrinkage weight for closeness & familiarity (`[0.0, 100.0]`).
    #[serde(default = "default_closeness_prior_count")]
    pub closeness_prior_count: f64,
    /// Lookahead horizon `H_debt` (ms) for imminent promises (`[0, 86_400_000]`).
    #[serde(default = "default_promise_debt_horizon_ms")]
    pub promise_debt_horizon_ms: u64,

    // --- Graded Addressivity & Social Cost (R-02, R-29) ---
    /// Closeness discount on `sel_other` (`[0.0, 1.0]`).
    #[serde(default = "default_lambda_c")]
    pub lambda_c: f64,
    /// Closeness discount on `answered` (`[0.0, 0.5]`).
    #[serde(default = "default_lambda_a")]
    pub lambda_a: f64,
    /// Commitment breaker threshold on `deg(e, soul)` (`[0.3, 1.0]`).
    #[serde(default = "default_theta_alloc")]
    pub theta_alloc: f64,
    /// Minimum degree `theta_ex` to log a closeness exchange (`[0.1, 1.0]`).
    #[serde(default = "default_theta_ex")]
    pub theta_ex: f64,
    /// Max sum-of-degrees audience size `M_ex` for dyadic closeness (`[1.0, 1000.0]`).
    #[serde(default = "default_m_ex")]
    pub m_ex: f64,
    /// Noisy-OR weight `w_c` on dyadic continuity (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_w_c")]
    pub w_c: f64,
    /// Noisy-OR weight `w_o` on embodied orientation (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_w_o")]
    pub w_o: f64,
    /// Per-turn continuity decay `gamma_c` (`[0.0, 1.0]`).
    #[serde(default = "default_gamma_c")]
    pub gamma_c: f64,
    /// Wall-clock continuity decay constant `tau_c` in seconds (`[1.0, 3600.0]`).
    #[serde(default = "default_tau_c_s")]
    pub tau_c_s: f64,
    /// Place continuity ring buffer length `W_c` (`[1, 64]`).
    #[serde(default = "default_w_c_window")]
    pub w_c_window: usize,
    /// Continuity saturation constant `K_c` (`[0.01, 5.0]`).
    #[serde(default = "default_k_c")]
    pub k_c: f64,
    /// Multi-partner fan penalty `beta_fan` (`[0.0, 2.0]`).
    #[serde(default = "default_beta_fan")]
    pub beta_fan: f64,
    /// Minimum past degree `theta_trace` for directed affinity (`[0.0, 1.0]`).
    #[serde(default = "default_theta_trace")]
    pub theta_trace: f64,

    // --- Backlog Chunking (R-14, A-04) ---
    /// Default place ingest-lag patience `patience_place` in ms (`[0, 86_400_000]`).
    #[serde(default = "default_patience_place_ms")]
    pub patience_place_ms: u64,
    /// Maximum place chunks appraised per wake (`[1, 6]`).
    #[serde(default = "default_backlog_max_chunks")]
    pub backlog_max_chunks: usize,

    // --- Idle Initiative & Breakpoint Timing (R-30) ---
    /// Threshold raise per `h_init` unit (`[0.0, 0.5]`, neutral `0.0`).
    #[serde(default = "default_step_init")]
    pub step_init: f64,
    /// Initiative habituation decay base per hour (`[0.05, 1.0]`).
    #[serde(default = "default_gamma_init")]
    pub gamma_init: f64,
    /// `h_init` increment on proactive emission (`[0.0, 3.0]`).
    #[serde(default = "default_delta_emit")]
    pub delta_emit: f64,
    /// `h_init` increment on ignored/held proactive emission (`[0.0, 5.0]`).
    #[serde(default = "default_delta_ign")]
    pub delta_ign: f64,
    /// Base interruption cost mid-burst `c_base` (`[0.0, 1.0]`).
    #[serde(default = "default_c_base")]
    pub c_base: f64,
    /// Breakpoint cost discount `beta_bp` (`[0.0, 1.0]`).
    #[serde(default = "default_beta_bp")]
    pub beta_bp: f64,
    /// Place burst-load cost coefficient `beta_load` (`[0.0, 1.0]`).
    #[serde(default = "default_beta_load")]
    pub beta_load: f64,
    /// Turn-pause multiplier over median latency `kappa_bp` (`[1.0, 5.0]`).
    #[serde(default = "default_kappa_bp")]
    pub kappa_bp: f64,
    /// Post-event-boundary receptivity window `tau_win` in seconds (`[1.0, 300.0]`).
    #[serde(default = "default_tau_win_s")]
    pub tau_win_s: f64,
    /// Sleep backlog penalty on `R-30` idle initiative (`[0.0, 1.0]`).
    #[serde(default = "default_lambda_sleep_init")]
    pub lambda_sleep_init: f64,

    // --- Opportunity-Cost Drives (R-31) ---
    /// Minimum unconsolidated events before sleep triggers (`[1, 1000]`).
    #[serde(default = "default_n_min_sleep")]
    pub n_min_sleep: usize,
    /// Sleep backlog vs wake-time weight `alpha_u` (`[0.0, 1.0]`).
    #[serde(default = "default_alpha_u")]
    pub alpha_u: f64,
    /// Wake-time sleep pressure time constant `tau_wake` in hours (`[1.0, 168.0]`).
    #[serde(default = "default_tau_wake_hours")]
    pub tau_wake_hours: f64,
    /// Maximum sleep speedup factor `D_sleep^max` on `T_idle` (`[1.0, 5.0]`).
    #[serde(default = "default_d_sleep_max")]
    pub d_sleep_max: f64,
    /// Switching horizon relative to `B_steps` (`[0.1, 2.0]`).
    #[serde(default = "default_tau_sw")]
    pub tau_sw: f64,
    /// Extra stagnation weight per repeated failure `w_fail` (`[0.0, 10.0]`).
    #[serde(default = "default_w_fail")]
    pub w_fail: f64,
    /// Switching resistance under high stakes `lambda_stakes` (`[0.0, 1.0]`).
    #[serde(default = "default_lambda_stakes")]
    pub lambda_stakes: f64,
    /// Switching trigger threshold `theta_sw` (`[0.1, 1.1]`).
    #[serde(default = "default_theta_sw")]
    pub theta_sw: f64,
    /// Switching budget compression weight `w_sw` (`[0.0, 1.0]`).
    #[serde(default = "default_w_sw")]
    pub w_sw: f64,
    /// Curiosity FoK inverted-U weight `lambda_fok` (`[0.0, 1.0]`).
    #[serde(default = "default_lambda_fok")]
    pub lambda_fok: f64,
    /// Curiosity expectation-surprise weight `lambda_surp` (`[0.0, 1.0]`).
    #[serde(default = "default_lambda_surp")]
    pub lambda_surp: f64,
    /// Curiosity Beta VoI weight `lambda_voi` (`[0.0, 1.0]`).
    #[serde(default = "default_lambda_voi")]
    pub lambda_voi: f64,
    /// EVC base control cost scaling `c0` (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_c0")]
    pub c0: f64,
    /// Sensitivity to queue/talk opportunity cost `gamma_opp` (`[0.0, 2.0]`).
    #[serde(default = "default_gamma_opp")]
    pub gamma_opp: f64,
    /// Super-linear EVC cost exponent `alpha_pow` (`[1.0, 3.0]`).
    #[serde(default = "default_alpha_pow")]
    pub alpha_pow: f64,
    /// Step marginal return decay `tau_k` (`[0.5, 10.0]`).
    #[serde(default = "default_tau_k")]
    pub tau_k: f64,
    /// Recall cost weight `w_rec` (`[0.0, 2.0]`).
    #[serde(default = "default_w_rec")]
    pub w_rec: f64,
    /// Opportunity-cost weight on sleep pressure `eta_sl` (`[0.0, 1.0]`).
    #[serde(default = "default_eta_sl")]
    pub eta_sl: f64,
    /// Opportunity-cost weight on talk rate `eta_tr` (`[0.0, 2.0]`).
    #[serde(default = "default_eta_tr")]
    pub eta_tr: f64,
    /// Reference queue size `Q_ref` for ambient opportunity cost (`[1.0, 1024.0]`).
    #[serde(default = "default_q_ref")]
    pub q_ref: f64,
    /// Default max deliberation steps `B_steps` (`[1, 64]`).
    #[serde(default = "default_b_steps")]
    pub b_steps: usize,
    /// Default max recall count `N_rec_max` (`[1, 32]`).
    #[serde(default = "default_n_rec_max")]
    pub n_rec_max: usize,

    // --- Shrunk Quantile Patience `q_0.10` (`A-08`) ---
    /// Lower bound `t_min` for patience wait window in ms (`[0, 600_000]`).
    #[serde(default = "default_patience_t_min_ms")]
    pub patience_t_min_ms: u64,
    /// Upper bound `t_max` for patience wait window in ms (`[0, 86_400_000]`).
    #[serde(default = "default_patience_t_max_ms")]
    pub patience_t_max_ms: u64,
    /// Body/global typical latency prior `L_typ` in ms (`[1, 86_400_000]`).
    #[serde(default = "default_patience_default_ms")]
    pub patience_default_ms: u64,
    /// Log-normal spread `sigma_{0,ln}` of prior CDF `F_0` (`[0.1, 3.0]`).
    #[serde(default = "default_patience_sigma_ln")]
    pub patience_sigma_ln: f64,
    /// Global/medium shrinkage pseudo-count `n_0` (`[0.1, 100.0]`).
    #[serde(default = "default_patience_prior_n0")]
    pub patience_prior_n0: f64,
    /// Place shrinkage pseudo-count `n_{0,place}` (`[0.1, 100.0]`).
    #[serde(default = "default_patience_place_n0")]
    pub patience_place_n0: f64,
    /// Target patience quantile `q_0.10` (`[0.01, 0.99]`).
    #[serde(default = "default_patience_quantile")]
    pub patience_quantile: f64,
    /// Per-observation exponential forgetting factor `rho_lat` (`[0.90, 1.0]`).
    #[serde(default = "default_patience_decay_rho")]
    pub patience_decay_rho: f64,

    // --- Cognitive Map SR & Bounded PPR (`R-32`) ---
    /// Co-occurrence edge hourly decay rate `lambda_map` (`[0.0, 1.0]`).
    #[serde(default = "default_map_lambda_decay")]
    pub map_lambda_decay: f64,
    /// Co-occurrence increment per event `eta_co` (`[0.0, 5.0]`).
    #[serde(default = "default_map_eta_co")]
    pub map_eta_co: f64,
    /// Co-occurrence weight cap `W_max` (`[1.0, 500.0]`).
    #[serde(default = "default_map_w_max")]
    pub map_w_max: f64,
    /// Successor Representation discount factor `gamma_sr` (`[0.0, 0.99]`).
    #[serde(default = "default_map_gamma_sr")]
    pub map_gamma_sr: f64,
    /// Successor Representation TD learning rate `alpha_sr` (`[0.0, 1.0]`).
    #[serde(default = "default_map_alpha_sr")]
    pub map_alpha_sr: f64,
    /// SR time-step normalization `tau_step` in seconds (`[1.0, 3600.0]`).
    #[serde(default = "default_map_tau_step_s")]
    pub map_tau_step_s: f64,
    /// Max intra-episode transition gap `Delta t_ep` in seconds (`[60.0, 86400.0]`).
    #[serde(default = "default_map_ep_gap_s")]
    pub map_ep_gap_s: f64,
    /// SR vs co-occurrence mixing weight `beta_sr` (`[0.0, 1.0]`).
    #[serde(default = "default_map_beta_sr")]
    pub map_beta_sr: f64,
    /// PPR damping factor `delta_ppr` (`[0.0, 0.95]`).
    #[serde(default = "default_map_delta_ppr")]
    pub map_delta_ppr: f64,
    /// Explicit multi-hop PPR iteration budget `L_ppr` (`[0, 16]`).
    #[serde(default = "default_map_hop_budget")]
    pub map_hop_budget: usize,
    /// Max non-zero SR / graph out-degree per node `K_sr` (`[1, 128]`).
    #[serde(default = "default_map_k_sr")]
    pub map_k_sr: usize,
    /// Cognitive map spreading activation weight `w_map` in recall (`[0.0, 2.0]`, neutral `0.0`).
    #[serde(default = "default_map_w_map")]
    pub map_w_map: f64,

    // --- Hybrid Recall Candidate Caps (`A-09`, `R-12`) ---
    /// Vector candidate pool limit per query tag (`[1, 200]`, default `40`).
    #[serde(default = "default_recall_vector_top_k")]
    pub recall_vector_top_k: usize,
    /// Text candidate pool limit per query (`[1, 200]`, default `20`).
    #[serde(default = "default_recall_text_top_k")]
    pub recall_text_top_k: usize,

    // --- Expectations, Retries, Agency, Grounding, Silence, Wake Gate (`R-07`..`R-10`, `R-18`, `R-19`, `R-42`, `R-49`) ---
    /// Maximum retry attempts on transient `failed`/`unknown` promise receipts (`[0, 10]`, `0` = no retry).
    #[serde(default = "default_promise_retry_max")]
    pub promise_retry_max: u32,
    /// Base exponential backoff delay `b` in ms (`[10, 3_600_000]`).
    #[serde(default = "default_promise_retry_base_ms")]
    pub promise_retry_base_ms: u64,
    /// Agency comparator tolerance multiplier `k` in `|delta_obs - delta_pred| <= k * sigma` (`[0.0, 10.0]`).
    #[serde(default = "default_agency_tolerance_k")]
    pub agency_tolerance_k: f64,
    /// Target quantile `q_e` for learned response prediction expiry (`[0.50, 0.99]`).
    #[serde(default = "default_response_expiry_quantile")]
    pub response_expiry_quantile: f64,
    /// Stakes multiplier on response expiry (`[1.0, 10.0]`).
    #[serde(default = "default_response_stakes_factor")]
    pub response_stakes_factor: f64,
    /// Z-score threshold `z_miss` for content-miss detection (`[0.5, 10.0]`).
    #[serde(default = "default_content_miss_z_threshold")]
    pub content_miss_z_threshold: f64,
    /// Minimum unrepaired reply samples in a place before z-score content-miss detection activates (`[1, 100]`).
    #[serde(default = "default_content_miss_min_samples")]
    pub content_miss_min_samples: usize,
    /// Seeded sampling propensity `rho` for judging `Silent` decisions (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_silence_sample_rate")]
    pub silence_sample_rate: f64,
    /// Initial lower cosine threshold `wake_gate_lo` for free-text conditions (`[0.0, 1.0]`).
    #[serde(default = "default_wake_gate_lo")]
    pub wake_gate_lo: f64,
    /// Rule-of-three error bound `epsilon` requiring `ceil(3 / epsilon)` clean verdicts (`[0.01, 0.50]`).
    #[serde(default = "default_wake_gate_hi_epsilon")]
    pub wake_gate_hi_epsilon: f64,
    /// Safety margin subtracted when calibrating `wake_gate_lo` (`[0.0, 0.50]`).
    #[serde(default = "default_wake_gate_margin")]
    pub wake_gate_margin: f64,
    /// Default goal expiry in ms (`[1_000, 2_592_000_000]`).
    #[serde(default = "default_default_goal_expiry_ms")]
    pub default_goal_expiry_ms: u64,
    /// Default `PendingApproval` expiry in ms (`[1_000, 86_400_000]`).
    #[serde(default = "default_pending_approval_expiry_ms")]
    pub pending_approval_expiry_ms: u64,

    // --- ContextPlan, Anchor Checks, Report Control, Progressive Tool RAG (`R-11`, `R-15`, `R-38`, `R-48`) ---
    /// Maximum byte budget for assembled `ContextPlan` volatile sections (`[512, 1_048_576]`).
    #[serde(default = "default_context_budget_bytes")]
    pub context_budget_bytes: usize,
    /// Maximum recent chain observations rendered in full before older ones become references (`[1, 128]`).
    #[serde(default = "default_working_set_full_obs_limit")]
    pub working_set_full_obs_limit: usize,
    /// Maximum recent place-tail events included in `ContextPlan` (`[1, 256]`).
    #[serde(default = "default_place_tail_limit")]
    pub place_tail_limit: usize,
    /// `[ANCHOR_CHECK]` register reset interval in chain wakes (`[1, 128]`, `R-48`).
    #[serde(default = "default_anchor_check_interval_wakes")]
    pub anchor_check_interval_wakes: u64,
    /// Maximum automatic `memory.recall` retries when calibrated confidence `< P_rc` (`[0, 8]`, `A-01`).
    #[serde(default = "default_auto_recall_max")]
    pub auto_recall_max: u8,
    /// Whether progressive `Tool RAG` disclosure bounds apply (`false` => neutral `k_cap = |C_avail|, k_proc = 0`).
    #[serde(default = "default_tool_rag_enabled")]
    pub tool_rag_enabled: bool,
    /// Dynamic capability disclosure cap `k_cap` (`[1, 1024]`, default `8`).
    #[serde(default = "default_tool_rag_k_cap")]
    pub tool_rag_k_cap: usize,
    /// Active procedure disclosure cap `k_proc` (`[0, 32]`, default `3`).
    #[serde(default = "default_tool_rag_k_proc")]
    pub tool_rag_k_proc: usize,
    /// Static `OwnerSigned` core tool cap in cacheable prefix (`[0, 16]`, default `4`).
    #[serde(default = "default_tool_rag_max_static_core")]
    pub tool_rag_max_static_core: usize,

    // --- Beliefs, Credence, Corroboration & Conventions (`R-20`, `R-32`, `R-43`) ---
    /// Beta credence longevity decay factor `lambda_domain` per period (`[0.0, 1.0]`, neutral `1.0`).
    #[serde(default = "default_credence_lambda_domain")]
    pub credence_lambda_domain: f64,
    /// Credence decay period in ms (`[60_000, 2_592_000_000]`, default 1 day).
    #[serde(default = "default_credence_period_ms")]
    pub credence_period_ms: u64,
    /// Minimum distinct corroborating actors `k_corrob` to elevate untrusted `told` claims (`[1, 10]`, default `2`).
    #[serde(default = "default_k_corrob")]
    pub k_corrob: usize,
    /// Minimum source Beta reliability to elevate a corroborated `told` claim (`[0.50, 0.99]`).
    #[serde(default = "default_rule_trust_threshold")]
    pub rule_trust_threshold: f64,
    /// Place convention anchored Beta prior `alpha_0^conv` (`[0.1, 10.0]`).
    #[serde(default = "default_conv_alpha_0")]
    pub conv_alpha_0: f64,
    /// Place convention anchored Beta prior `beta_0^conv` (`[0.1, 10.0]`).
    #[serde(default = "default_conv_beta_0")]
    pub conv_beta_0: f64,
    /// Place convention per-period retention factor `lambda_conv` (`[0.0, 1.0]`).
    #[serde(default = "default_conv_lambda")]
    pub conv_lambda: f64,
    /// Vicarious peer convention observation discount `lambda_vic` (`[0.0, 1.0]`, neutral `0.0`).
    #[serde(default = "default_conv_vicarious_discount")]
    pub conv_vicarious_discount: f64,
    /// Peer reliability floor `w_min` for convention observation weight (`[0.0, 1.0]`).
    #[serde(default = "default_conv_peer_floor")]
    pub conv_peer_floor: f64,
    /// Convention effective-sample shrinkage constant `tau_conv` (`[0.5, 50.0]`).
    #[serde(default = "default_conv_tau")]
    pub conv_tau: f64,
    /// Convention logit preference gain `kappa_conv` (`[0.0, 2.0]`).
    #[serde(default = "default_conv_kappa")]
    pub conv_kappa: f64,
    /// Convention bias clamp `B_conv` (`[0.0, 2.0]`, neutral `0.0`).
    #[serde(default = "default_conv_bias_max")]
    pub conv_bias_max: f64,

    // --- Sleep, Episodes, EM-LLM Boundaries & Merge (`A-04`..`A-06`, `R-24`) ---
    /// Global idle duration `T_idle` in ms before sleep triggers (`[1_000, 86_400_000]`).
    #[serde(default = "default_t_idle_sleep_ms")]
    pub t_idle_sleep_ms: u64,
    /// Enable `{open, provisional, sealed}` soft-closing of idle tails (`false` = `sealed` only).
    #[serde(default = "default_provisional_close")]
    pub provisional_close: bool,
    /// Minimum events `N_ep_min` to soft-close an idle open tail (`[1, 64]`).
    #[serde(default = "default_n_ep_min")]
    pub n_ep_min: usize,
    /// Hard maximum events `N_ep_max` per episode (`[4, 1024]`).
    #[serde(default = "default_n_ep_max")]
    pub n_ep_max: usize,
    /// Staleness multiplier `k_stale` over `T_idle` (`[1.0, 48.0]`).
    #[serde(default = "default_k_stale_sleep")]
    pub k_stale_sleep: f64,
    /// EM-LLM spike multiplier `gamma` (`[0.1, 5.0]`).
    #[serde(default = "default_em_boundary_gamma")]
    pub em_boundary_gamma: f64,
    /// EM-LLM causal sliding window size `w` (`[5, 200]`).
    #[serde(default = "default_em_boundary_window")]
    pub em_boundary_window: usize,
    /// EM-LLM warmup event count `w_min` (`[1, 50]`).
    #[serde(default = "default_em_boundary_warmup")]
    pub em_boundary_warmup: usize,
    /// Empirical-Bayes prior surprise mean `mu_0` (`[0.0, 1.0]`).
    #[serde(default = "default_em_boundary_mu0")]
    pub em_boundary_mu0: f64,
    /// Empirical-Bayes prior surprise std `sigma_0` (`[0.01, 1.0]`).
    #[serde(default = "default_em_boundary_sigma0")]
    pub em_boundary_sigma0: f64,
    /// Empirical-Bayes shrinkage pseudo-count `m_0` (`[0.0, 50.0]`).
    #[serde(default = "default_em_boundary_m0")]
    pub em_boundary_m0: f64,
    /// Positive temporal z-score clamp `z_max` (`[1.0, 10.0]`).
    #[serde(default = "default_em_boundary_z_max")]
    pub em_boundary_z_max: f64,
    /// Log-gap standard deviation floor `sigma_{u,floor}` (`[0.05, 2.0]`).
    #[serde(default = "default_em_boundary_sigma_u_floor")]
    pub em_boundary_sigma_u_floor: f64,
    /// Neutral fixed-count boundary interval when `switches.event_boundaries == false` (`[2, 256]`).
    #[serde(default = "default_em_fixed_interval")]
    pub em_fixed_interval: usize,
    /// Minimum participant Jaccard overlap `J` for episode merge (`[0.5, 1.0]`).
    #[serde(default = "default_merge_jaccard_min")]
    pub merge_jaccard_min: f64,
    /// Maximum inter-episode gap in ms for adjacent merge (`[0, 86_400_000]`).
    #[serde(default = "default_merge_gap_ms")]
    pub merge_gap_ms: u64,

    // --- Domain Clustering & Hungarian Alignment (`A-12`) ---
    /// Maximum domain centroids `K` (`[1, 64]`, `1` = single global domain `d_0`).
    #[serde(default = "default_domain_clusters_k")]
    pub domain_clusters_k: usize,
    /// Minimum active vectors per cluster `m_min` (`[1, 50]`).
    #[serde(default = "default_domain_m_min")]
    pub domain_m_min: usize,
    /// Spherical k-means `max_iter` (`[5, 100]`).
    #[serde(default = "default_domain_max_iter")]
    pub domain_max_iter: usize,
    /// Evolutionary history pseudo-count `w_hist` (`[0.0, 20.0]`, neutral `0.0`).
    #[serde(default = "default_domain_w_hist")]
    pub domain_w_hist: f64,
    /// Minimum cosine for Hungarian domain ID continuation (`[0.30, 0.99]`).
    #[serde(default = "default_domain_tau_match")]
    pub domain_tau_match: f64,
    /// Minimum cosine to mark unmatched new centroid as a split (`[0.10, 0.95]`).
    #[serde(default = "default_domain_tau_rel")]
    pub domain_tau_rel: f64,
    /// Minimum cosine to merge unmatched old centroid into survivor (`[0.40, 0.99]`).
    #[serde(default = "default_domain_tau_merge")]
    pub domain_tau_merge: f64,
    /// Cosine drift discount exponent `gamma_drift` (`[0.0, 2.0]`, neutral `0.0`).
    #[serde(default = "default_domain_gamma_drift")]
    pub domain_gamma_drift: f64,
    /// Residual faded evidence split inheritance fraction `rho_split` (`[0.0, 0.5]`, default `0.0`).
    #[serde(default = "default_domain_rho_split")]
    pub domain_rho_split: f64,

    // --- Lessons, Procedures & CUSUM Drift (`A-11`, `R-38`) ---
    /// Minimum distinct failure episodes to form a contrastive lesson (`[2, 20]`).
    #[serde(default = "default_lesson_min_episodes")]
    pub lesson_min_episodes: usize,
    /// Maximum contrastive success traces attached to a lesson (`[1, 10]`).
    #[serde(default = "default_lesson_max_contrasts")]
    pub lesson_max_contrasts: usize,
    /// Per-execution Beta decay factor `gamma_beta` for procedures (`[0.50, 0.999]`).
    #[serde(default = "default_proc_gamma_beta")]
    pub proc_gamma_beta: f64,
    /// Minimum executions before locking procedure CUSUM baseline `mu_0` (`[2, 50]`).
    #[serde(default = "default_proc_cusum_n_min_lock")]
    pub proc_cusum_n_min_lock: usize,
    /// Downward Bernoulli CUSUM reference allowance `k_drift` (`[0.01, 0.50]`).
    #[serde(default = "default_proc_cusum_k_drift")]
    pub proc_cusum_k_drift: f64,
    /// Downward CUSUM warning threshold `h_warn` zeroing habit strength (`[0.5, 10.0]`).
    #[serde(default = "default_proc_cusum_h_warn")]
    pub proc_cusum_h_warn: f64,
    /// Downward CUSUM soft-retirement threshold `h_retire` (`[1.0, 20.0]`).
    #[serde(default = "default_proc_cusum_h_retire")]
    pub proc_cusum_h_retire: f64,
    /// Posterior UCB floor triggering soft retirement (`[0.05, 0.70]`).
    #[serde(default = "default_proc_theta_retire")]
    pub proc_theta_retire: f64,
    /// Invariant: chat-body habit bypass of reasoning is disabled (`false`).
    #[serde(default = "default_allow_chat_habit_bypass")]
    pub allow_chat_habit_bypass: bool,

    // --- Perception Ledger & Four-Pillar `self_card` (`R-25`, `R-35`) ---
    /// Maximum tracked items per actor ledger card (`[4, 256]`).
    #[serde(default = "default_ledger_max_items_per_card")]
    pub ledger_max_items_per_card: usize,
    /// Capability LCB z-score `z_cap` (`[0.5, 4.0]`).
    #[serde(default = "default_self_card_cap_z")]
    pub self_card_cap_z: f64,
    /// Minimum trials before capability LCB degradation triggers (`[1, 50]`).
    #[serde(default = "default_self_card_cap_min_trials")]
    pub self_card_cap_min_trials: usize,
    /// Capability LCB floor flagging `degraded_c` (`[0.05, 0.90]`).
    #[serde(default = "default_self_card_cap_deg_threshold")]
    pub self_card_cap_deg_threshold: f64,
    /// Consecutive capability failures flagging `degraded_c` (`[1, 20]`).
    #[serde(default = "default_self_card_cap_consec_fail")]
    pub self_card_cap_consec_fail: u32,
    /// Logit bias-correction gain `kappa_bias` (`[0.0, 5.0]`).
    #[serde(default = "default_self_card_kappa_bias")]
    pub self_card_kappa_bias: f64,
    /// Calibration shrinkage pseudo-count `N_{cal,0}` (`[0.5, 50.0]`).
    #[serde(default = "default_self_card_cal_n0")]
    pub self_card_cal_n0: f64,

    // --- Bounded Learning, IPS & Quantile Calibration (`R-19`, `R-26`, `A-13`) ---
    /// Fast salience weight learning rate `alpha_w` (`[0.0, 0.25]`, neutral `0.0`).
    #[serde(default = "default_learn_rate_w")]
    pub learn_rate_w: f64,
    /// Slow threshold learning-rate ratio `alpha_theta / alpha_w` (`[0.0, 1.0]`).
    #[serde(default = "default_learn_rate_theta_ratio")]
    pub learn_rate_theta_ratio: f64,
    /// Maximum daily fractional weight move cap (`[0.0, 0.50]`).
    #[serde(default = "default_learn_daily_max_delta")]
    pub learn_daily_max_delta: f64,
    /// Clipped IPS upper bound `M_ips` for sampled silences (`[1.0, 100.0]`).
    #[serde(default = "default_learn_ips_clip_m")]
    pub learn_ips_clip_m: f64,
    /// Shrinkage pseudo-count `n_0` for `q_0.10` `wake_gate_lo` calibration (`[0.5, 50.0]`).
    #[serde(default = "default_learn_wake_gate_n0")]
    pub learn_wake_gate_n0: f64,
    /// Target lower quantile `q_0.10` for `wake_gate_lo` (`[0.01, 0.50]`).
    #[serde(default = "default_learn_wake_gate_quantile")]
    pub learn_wake_gate_quantile: f64,
}

impl Default for Prior {
    fn default() -> Self {
        Self {
            version: CURRENT_PRIOR_FORMAT_VERSION,
            content_version: "2026-11-16.1".to_owned(),
            talkativeness: 0.5,
            spontaneity: 0.5,
            memory_persistence: 0.5,
            helpfulness: 0.5,
            restraint: 0.5,
            boredom_with_repeats: 0.5,
            certainty_bar: 0.5,
            speaking_bar: 3,
            own_name_weight: default_own_name_weight(),
            info_scale: default_info_scale(),
            fok_scale: default_fok_scale(),
            relevance_weight: default_relevance_weight(),
            commitment_weight: default_commitment_weight(),
            restraint_scale: default_restraint_scale(),
            asr_conf_threshold: default_asr_conf_threshold(),
            asr_clarification_boost: default_asr_clarification_boost(),
            w_b: default_w_b(),
            context_weight: default_context_weight(),
            importance_weight: default_importance_weight(),
            fok_threshold: default_fok_threshold(),
            fok_scale_norm: default_fok_scale_norm(),
            fade_threshold: default_fade_threshold(),
            kappa: default_kappa(),
            a_scale: default_a_scale(),
            budget_weight: default_budget_weight(),
            promise_debt_weight: default_promise_debt_weight(),
            reserve_when_unfamiliar: default_reserve_when_unfamiliar(),
            note_band: default_note_band(),
            talk_rate_window_minutes: default_talk_rate_window_minutes(),
            talk_rate_prior_count: default_talk_rate_prior_count(),
            closeness_prior_count: default_closeness_prior_count(),
            promise_debt_horizon_ms: default_promise_debt_horizon_ms(),
            lambda_c: default_lambda_c(),
            lambda_a: default_lambda_a(),
            theta_alloc: default_theta_alloc(),
            theta_ex: default_theta_ex(),
            m_ex: default_m_ex(),
            w_c: default_w_c(),
            w_o: default_w_o(),
            gamma_c: default_gamma_c(),
            tau_c_s: default_tau_c_s(),
            w_c_window: default_w_c_window(),
            k_c: default_k_c(),
            beta_fan: default_beta_fan(),
            theta_trace: default_theta_trace(),
            patience_place_ms: default_patience_place_ms(),
            backlog_max_chunks: default_backlog_max_chunks(),
            step_init: default_step_init(),
            gamma_init: default_gamma_init(),
            delta_emit: default_delta_emit(),
            delta_ign: default_delta_ign(),
            c_base: default_c_base(),
            beta_bp: default_beta_bp(),
            beta_load: default_beta_load(),
            kappa_bp: default_kappa_bp(),
            tau_win_s: default_tau_win_s(),
            lambda_sleep_init: default_lambda_sleep_init(),
            n_min_sleep: default_n_min_sleep(),
            alpha_u: default_alpha_u(),
            tau_wake_hours: default_tau_wake_hours(),
            d_sleep_max: default_d_sleep_max(),
            tau_sw: default_tau_sw(),
            w_fail: default_w_fail(),
            lambda_stakes: default_lambda_stakes(),
            theta_sw: default_theta_sw(),
            w_sw: default_w_sw(),
            lambda_fok: default_lambda_fok(),
            lambda_surp: default_lambda_surp(),
            lambda_voi: default_lambda_voi(),
            c0: default_c0(),
            gamma_opp: default_gamma_opp(),
            alpha_pow: default_alpha_pow(),
            tau_k: default_tau_k(),
            w_rec: default_w_rec(),
            eta_sl: default_eta_sl(),
            eta_tr: default_eta_tr(),
            q_ref: default_q_ref(),
            b_steps: default_b_steps(),
            n_rec_max: default_n_rec_max(),
            patience_t_min_ms: default_patience_t_min_ms(),
            patience_t_max_ms: default_patience_t_max_ms(),
            patience_default_ms: default_patience_default_ms(),
            patience_sigma_ln: default_patience_sigma_ln(),
            patience_prior_n0: default_patience_prior_n0(),
            patience_place_n0: default_patience_place_n0(),
            patience_quantile: default_patience_quantile(),
            patience_decay_rho: default_patience_decay_rho(),
            map_lambda_decay: default_map_lambda_decay(),
            map_eta_co: default_map_eta_co(),
            map_w_max: default_map_w_max(),
            map_gamma_sr: default_map_gamma_sr(),
            map_alpha_sr: default_map_alpha_sr(),
            map_tau_step_s: default_map_tau_step_s(),
            map_ep_gap_s: default_map_ep_gap_s(),
            map_beta_sr: default_map_beta_sr(),
            map_delta_ppr: default_map_delta_ppr(),
            map_hop_budget: default_map_hop_budget(),
            map_k_sr: default_map_k_sr(),
            map_w_map: default_map_w_map(),
            recall_vector_top_k: default_recall_vector_top_k(),
            recall_text_top_k: default_recall_text_top_k(),
            promise_retry_max: default_promise_retry_max(),
            promise_retry_base_ms: default_promise_retry_base_ms(),
            agency_tolerance_k: default_agency_tolerance_k(),
            response_expiry_quantile: default_response_expiry_quantile(),
            response_stakes_factor: default_response_stakes_factor(),
            content_miss_z_threshold: default_content_miss_z_threshold(),
            content_miss_min_samples: default_content_miss_min_samples(),
            silence_sample_rate: default_silence_sample_rate(),
            wake_gate_lo: default_wake_gate_lo(),
            wake_gate_hi_epsilon: default_wake_gate_hi_epsilon(),
            wake_gate_margin: default_wake_gate_margin(),
            default_goal_expiry_ms: default_default_goal_expiry_ms(),
            pending_approval_expiry_ms: default_pending_approval_expiry_ms(),
            context_budget_bytes: default_context_budget_bytes(),
            working_set_full_obs_limit: default_working_set_full_obs_limit(),
            place_tail_limit: default_place_tail_limit(),
            anchor_check_interval_wakes: default_anchor_check_interval_wakes(),
            auto_recall_max: default_auto_recall_max(),
            tool_rag_enabled: default_tool_rag_enabled(),
            tool_rag_k_cap: default_tool_rag_k_cap(),
            tool_rag_k_proc: default_tool_rag_k_proc(),
            tool_rag_max_static_core: default_tool_rag_max_static_core(),
            credence_lambda_domain: default_credence_lambda_domain(),
            credence_period_ms: default_credence_period_ms(),
            k_corrob: default_k_corrob(),
            rule_trust_threshold: default_rule_trust_threshold(),
            conv_alpha_0: default_conv_alpha_0(),
            conv_beta_0: default_conv_beta_0(),
            conv_lambda: default_conv_lambda(),
            conv_vicarious_discount: default_conv_vicarious_discount(),
            conv_peer_floor: default_conv_peer_floor(),
            conv_tau: default_conv_tau(),
            conv_kappa: default_conv_kappa(),
            conv_bias_max: default_conv_bias_max(),
            t_idle_sleep_ms: default_t_idle_sleep_ms(),
            provisional_close: default_provisional_close(),
            n_ep_min: default_n_ep_min(),
            n_ep_max: default_n_ep_max(),
            k_stale_sleep: default_k_stale_sleep(),
            em_boundary_gamma: default_em_boundary_gamma(),
            em_boundary_window: default_em_boundary_window(),
            em_boundary_warmup: default_em_boundary_warmup(),
            em_boundary_mu0: default_em_boundary_mu0(),
            em_boundary_sigma0: default_em_boundary_sigma0(),
            em_boundary_m0: default_em_boundary_m0(),
            em_boundary_z_max: default_em_boundary_z_max(),
            em_boundary_sigma_u_floor: default_em_boundary_sigma_u_floor(),
            em_fixed_interval: default_em_fixed_interval(),
            merge_jaccard_min: default_merge_jaccard_min(),
            merge_gap_ms: default_merge_gap_ms(),
            domain_clusters_k: default_domain_clusters_k(),
            domain_m_min: default_domain_m_min(),
            domain_max_iter: default_domain_max_iter(),
            domain_w_hist: default_domain_w_hist(),
            domain_tau_match: default_domain_tau_match(),
            domain_tau_rel: default_domain_tau_rel(),
            domain_tau_merge: default_domain_tau_merge(),
            domain_gamma_drift: default_domain_gamma_drift(),
            domain_rho_split: default_domain_rho_split(),
            lesson_min_episodes: default_lesson_min_episodes(),
            lesson_max_contrasts: default_lesson_max_contrasts(),
            proc_gamma_beta: default_proc_gamma_beta(),
            proc_cusum_n_min_lock: default_proc_cusum_n_min_lock(),
            proc_cusum_k_drift: default_proc_cusum_k_drift(),
            proc_cusum_h_warn: default_proc_cusum_h_warn(),
            proc_cusum_h_retire: default_proc_cusum_h_retire(),
            proc_theta_retire: default_proc_theta_retire(),
            allow_chat_habit_bypass: default_allow_chat_habit_bypass(),
            ledger_max_items_per_card: default_ledger_max_items_per_card(),
            self_card_cap_z: default_self_card_cap_z(),
            self_card_cap_min_trials: default_self_card_cap_min_trials(),
            self_card_cap_deg_threshold: default_self_card_cap_deg_threshold(),
            self_card_cap_consec_fail: default_self_card_cap_consec_fail(),
            self_card_kappa_bias: default_self_card_kappa_bias(),
            self_card_cal_n0: default_self_card_cal_n0(),
            learn_rate_w: default_learn_rate_w(),
            learn_rate_theta_ratio: default_learn_rate_theta_ratio(),
            learn_daily_max_delta: default_learn_daily_max_delta(),
            learn_ips_clip_m: default_learn_ips_clip_m(),
            learn_wake_gate_n0: default_learn_wake_gate_n0(),
            learn_wake_gate_quantile: default_learn_wake_gate_quantile(),
        }
    }
}

impl Prior {
    /// Validates format version, content version, and all disposition bounds.
    pub fn validate(&self) -> Result<(), PriorError> {
        if self.version == 0 || self.version > CURRENT_PRIOR_FORMAT_VERSION {
            return Err(PriorError::UnsupportedFormatVersion {
                found: self.version,
                supported: CURRENT_PRIOR_FORMAT_VERSION,
            });
        }
        if self.content_version.trim().is_empty() {
            return Err(PriorError::EmptyContentVersion);
        }
        check_unit("talkativeness", self.talkativeness)?;
        check_unit("spontaneity", self.spontaneity)?;
        check_unit("memory_persistence", self.memory_persistence)?;
        check_unit("helpfulness", self.helpfulness)?;
        check_unit("restraint", self.restraint)?;
        check_unit("boredom_with_repeats", self.boredom_with_repeats)?;
        check_unit("certainty_bar", self.certainty_bar)?;
        if !(1..=5).contains(&self.speaking_bar) {
            return Err(PriorError::OutOfRange {
                disposition: "speaking_bar",
                value: self.speaking_bar.to_string(),
                range: "[1, 5]",
            });
        }

        check_unit("own_name_weight", self.own_name_weight)?;
        check_unit("info_scale", self.info_scale)?;
        check_unit("fok_scale", self.fok_scale)?;
        check_unit("relevance_weight", self.relevance_weight)?;
        check_unit("commitment_weight", self.commitment_weight)?;
        check_unit("restraint_scale", self.restraint_scale)?;
        check_unit("asr_conf_threshold", self.asr_conf_threshold)?;
        check_unit("asr_clarification_boost", self.asr_clarification_boost)?;

        check_range("w_b", self.w_b, 0.0, 2.0, "[0.0, 2.0]")?;
        check_range(
            "context_weight",
            self.context_weight,
            0.0,
            3.0,
            "[0.0, 3.0]",
        )?;
        check_range(
            "importance_weight",
            self.importance_weight,
            0.0,
            2.0,
            "[0.0, 2.0]",
        )?;
        check_range(
            "fok_threshold",
            self.fok_threshold,
            -5.0,
            5.0,
            "[-5.0, 5.0]",
        )?;
        check_range(
            "fok_scale_norm",
            self.fok_scale_norm,
            0.05,
            5.0,
            "[0.05, 5.0]",
        )?;
        check_range(
            "fade_threshold",
            self.fade_threshold,
            -10.0,
            0.0,
            "[-10.0, 0.0]",
        )?;

        check_range("kappa", self.kappa, 0.0, 2.0, "[0.0, 2.0]")?;
        check_range("a_scale", self.a_scale, 0.0, 1.5, "[0.0, 1.5]")?;
        check_range("budget_weight", self.budget_weight, 0.0, 0.6, "[0.0, 0.6]")?;
        check_range(
            "promise_debt_weight",
            self.promise_debt_weight,
            0.0,
            0.5,
            "[0.0, 0.5]",
        )?;
        check_range(
            "reserve_when_unfamiliar",
            self.reserve_when_unfamiliar,
            0.0,
            0.3,
            "[0.0, 0.3]",
        )?;
        check_range("note_band", self.note_band, 0.01, 0.3, "[0.01, 0.3]")?;
        check_range(
            "talk_rate_window_minutes",
            self.talk_rate_window_minutes,
            5.0,
            120.0,
            "[5.0, 120.0]",
        )?;
        check_range(
            "talk_rate_prior_count",
            self.talk_rate_prior_count,
            0.0,
            100.0,
            "[0.0, 100.0]",
        )?;
        check_range(
            "closeness_prior_count",
            self.closeness_prior_count,
            0.0,
            100.0,
            "[0.0, 100.0]",
        )?;
        check_u64_range(
            "promise_debt_horizon_ms",
            self.promise_debt_horizon_ms,
            0,
            86_400_000,
            "[0, 86400000]",
        )?;

        check_unit("lambda_c", self.lambda_c)?;
        check_range("lambda_a", self.lambda_a, 0.0, 0.5, "[0.0, 0.5]")?;
        check_range("theta_alloc", self.theta_alloc, 0.3, 1.0, "[0.3, 1.0]")?;
        check_range("theta_ex", self.theta_ex, 0.1, 1.0, "[0.1, 1.0]")?;
        check_range("m_ex", self.m_ex, 1.0, 1000.0, "[1.0, 1000.0]")?;
        check_unit("w_c", self.w_c)?;
        check_unit("w_o", self.w_o)?;
        check_unit("gamma_c", self.gamma_c)?;
        check_range("tau_c_s", self.tau_c_s, 1.0, 3600.0, "[1.0, 3600.0]")?;
        check_usize_range("w_c_window", self.w_c_window, 1, 64, "[1, 64]")?;
        check_range("k_c", self.k_c, 0.01, 5.0, "[0.01, 5.0]")?;
        check_range("beta_fan", self.beta_fan, 0.0, 2.0, "[0.0, 2.0]")?;
        check_unit("theta_trace", self.theta_trace)?;

        check_u64_range(
            "patience_place_ms",
            self.patience_place_ms,
            0,
            86_400_000,
            "[0, 86400000]",
        )?;
        check_usize_range(
            "backlog_max_chunks",
            self.backlog_max_chunks,
            1,
            6,
            "[1, 6]",
        )?;

        check_range("step_init", self.step_init, 0.0, 0.5, "[0.0, 0.5]")?;
        check_range("gamma_init", self.gamma_init, 0.05, 1.0, "[0.05, 1.0]")?;
        check_range("delta_emit", self.delta_emit, 0.0, 3.0, "[0.0, 3.0]")?;
        check_range("delta_ign", self.delta_ign, 0.0, 5.0, "[0.0, 5.0]")?;
        check_unit("c_base", self.c_base)?;
        check_unit("beta_bp", self.beta_bp)?;
        check_unit("beta_load", self.beta_load)?;
        check_range("kappa_bp", self.kappa_bp, 1.0, 5.0, "[1.0, 5.0]")?;
        check_range("tau_win_s", self.tau_win_s, 1.0, 300.0, "[1.0, 300.0]")?;
        check_unit("lambda_sleep_init", self.lambda_sleep_init)?;

        check_usize_range("n_min_sleep", self.n_min_sleep, 1, 1000, "[1, 1000]")?;
        check_unit("alpha_u", self.alpha_u)?;
        check_range(
            "tau_wake_hours",
            self.tau_wake_hours,
            1.0,
            168.0,
            "[1.0, 168.0]",
        )?;
        check_range("d_sleep_max", self.d_sleep_max, 1.0, 5.0, "[1.0, 5.0]")?;
        check_range("tau_sw", self.tau_sw, 0.1, 2.0, "[0.1, 2.0]")?;
        check_range("w_fail", self.w_fail, 0.0, 10.0, "[0.0, 10.0]")?;
        check_unit("lambda_stakes", self.lambda_stakes)?;
        check_range("theta_sw", self.theta_sw, 0.1, 1.1, "[0.1, 1.1]")?;
        check_unit("w_sw", self.w_sw)?;
        check_unit("lambda_fok", self.lambda_fok)?;
        check_unit("lambda_surp", self.lambda_surp)?;
        check_unit("lambda_voi", self.lambda_voi)?;
        check_unit("c0", self.c0)?;
        check_range("gamma_opp", self.gamma_opp, 0.0, 2.0, "[0.0, 2.0]")?;
        check_range("alpha_pow", self.alpha_pow, 1.0, 3.0, "[1.0, 3.0]")?;
        check_range("tau_k", self.tau_k, 0.5, 10.0, "[0.5, 10.0]")?;
        check_range("w_rec", self.w_rec, 0.0, 2.0, "[0.0, 2.0]")?;
        check_unit("eta_sl", self.eta_sl)?;
        check_range("eta_tr", self.eta_tr, 0.0, 2.0, "[0.0, 2.0]")?;
        check_range("q_ref", self.q_ref, 1.0, 1024.0, "[1.0, 1024.0]")?;
        check_usize_range("b_steps", self.b_steps, 1, 64, "[1, 64]")?;
        check_usize_range("n_rec_max", self.n_rec_max, 1, 32, "[1, 32]")?;

        check_u64_range(
            "patience_t_min_ms",
            self.patience_t_min_ms,
            0,
            600_000,
            "[0, 600000]",
        )?;
        check_u64_range(
            "patience_t_max_ms",
            self.patience_t_max_ms,
            0,
            86_400_000,
            "[0, 86400000]",
        )?;
        if self.patience_t_min_ms > self.patience_t_max_ms {
            return Err(PriorError::OutOfRange {
                disposition: "patience_t_min_ms",
                value: self.patience_t_min_ms.to_string(),
                range: "[0, patience_t_max_ms]",
            });
        }
        check_u64_range(
            "patience_default_ms",
            self.patience_default_ms,
            1,
            86_400_000,
            "[1, 86400000]",
        )?;
        check_range(
            "patience_sigma_ln",
            self.patience_sigma_ln,
            0.1,
            3.0,
            "[0.1, 3.0]",
        )?;
        check_range(
            "patience_prior_n0",
            self.patience_prior_n0,
            0.1,
            100.0,
            "[0.1, 100.0]",
        )?;
        check_range(
            "patience_place_n0",
            self.patience_place_n0,
            0.1,
            100.0,
            "[0.1, 100.0]",
        )?;
        check_range(
            "patience_quantile",
            self.patience_quantile,
            0.01,
            0.99,
            "[0.01, 0.99]",
        )?;
        check_range(
            "patience_decay_rho",
            self.patience_decay_rho,
            0.90,
            1.0,
            "[0.90, 1.0]",
        )?;

        check_unit("map_lambda_decay", self.map_lambda_decay)?;
        check_range("map_eta_co", self.map_eta_co, 0.0, 5.0, "[0.0, 5.0]")?;
        check_range("map_w_max", self.map_w_max, 1.0, 500.0, "[1.0, 500.0]")?;
        check_range("map_gamma_sr", self.map_gamma_sr, 0.0, 0.99, "[0.0, 0.99]")?;
        check_unit("map_alpha_sr", self.map_alpha_sr)?;
        check_range(
            "map_tau_step_s",
            self.map_tau_step_s,
            1.0,
            3600.0,
            "[1.0, 3600.0]",
        )?;
        check_range(
            "map_ep_gap_s",
            self.map_ep_gap_s,
            60.0,
            86_400.0,
            "[60.0, 86400.0]",
        )?;
        check_unit("map_beta_sr", self.map_beta_sr)?;
        check_range(
            "map_delta_ppr",
            self.map_delta_ppr,
            0.0,
            0.95,
            "[0.0, 0.95]",
        )?;
        check_usize_range("map_hop_budget", self.map_hop_budget, 0, 16, "[0, 16]")?;
        check_usize_range("map_k_sr", self.map_k_sr, 1, 128, "[1, 128]")?;
        check_range("map_w_map", self.map_w_map, 0.0, 2.0, "[0.0, 2.0]")?;

        check_usize_range(
            "recall_vector_top_k",
            self.recall_vector_top_k,
            1,
            200,
            "[1, 200]",
        )?;
        check_usize_range(
            "recall_text_top_k",
            self.recall_text_top_k,
            1,
            200,
            "[1, 200]",
        )?;

        check_u32_range(
            "promise_retry_max",
            self.promise_retry_max,
            0,
            10,
            "[0, 10]",
        )?;
        check_u64_range(
            "promise_retry_base_ms",
            self.promise_retry_base_ms,
            10,
            3_600_000,
            "[10, 3600000]",
        )?;
        check_range(
            "agency_tolerance_k",
            self.agency_tolerance_k,
            0.0,
            10.0,
            "[0.0, 10.0]",
        )?;
        check_range(
            "response_expiry_quantile",
            self.response_expiry_quantile,
            0.50,
            0.99,
            "[0.50, 0.99]",
        )?;
        check_range(
            "response_stakes_factor",
            self.response_stakes_factor,
            1.0,
            10.0,
            "[1.0, 10.0]",
        )?;
        check_range(
            "content_miss_z_threshold",
            self.content_miss_z_threshold,
            0.5,
            10.0,
            "[0.5, 10.0]",
        )?;
        check_usize_range(
            "content_miss_min_samples",
            self.content_miss_min_samples,
            1,
            100,
            "[1, 100]",
        )?;
        check_unit("silence_sample_rate", self.silence_sample_rate)?;
        check_unit("wake_gate_lo", self.wake_gate_lo)?;
        check_range(
            "wake_gate_hi_epsilon",
            self.wake_gate_hi_epsilon,
            0.01,
            0.50,
            "[0.01, 0.50]",
        )?;
        check_range(
            "wake_gate_margin",
            self.wake_gate_margin,
            0.0,
            0.50,
            "[0.0, 0.50]",
        )?;
        check_u64_range(
            "default_goal_expiry_ms",
            self.default_goal_expiry_ms,
            1_000,
            2_592_000_000,
            "[1000, 2592000000]",
        )?;
        check_u64_range(
            "pending_approval_expiry_ms",
            self.pending_approval_expiry_ms,
            1_000,
            86_400_000,
            "[1000, 86400000]",
        )?;

        check_usize_range(
            "context_budget_bytes",
            self.context_budget_bytes,
            512,
            1_048_576,
            "[512, 1048576]",
        )?;
        check_usize_range(
            "working_set_full_obs_limit",
            self.working_set_full_obs_limit,
            1,
            128,
            "[1, 128]",
        )?;
        check_usize_range(
            "place_tail_limit",
            self.place_tail_limit,
            1,
            256,
            "[1, 256]",
        )?;
        check_u64_range(
            "anchor_check_interval_wakes",
            self.anchor_check_interval_wakes,
            1,
            128,
            "[1, 128]",
        )?;
        if self.auto_recall_max > 8 {
            return Err(PriorError::OutOfRange {
                disposition: "auto_recall_max",
                value: self.auto_recall_max.to_string(),
                range: "[0, 8]",
            });
        }
        check_usize_range("tool_rag_k_cap", self.tool_rag_k_cap, 1, 1024, "[1, 1024]")?;
        check_usize_range("tool_rag_k_proc", self.tool_rag_k_proc, 0, 32, "[0, 32]")?;
        check_usize_range(
            "tool_rag_max_static_core",
            self.tool_rag_max_static_core,
            0,
            16,
            "[0, 16]",
        )?;

        check_unit("credence_lambda_domain", self.credence_lambda_domain)?;
        check_u64_range(
            "credence_period_ms",
            self.credence_period_ms,
            60_000,
            2_592_000_000,
            "[60000, 2592000000]",
        )?;
        check_usize_range("k_corrob", self.k_corrob, 1, 10, "[1, 10]")?;
        check_range(
            "rule_trust_threshold",
            self.rule_trust_threshold,
            0.50,
            0.99,
            "[0.50, 0.99]",
        )?;
        check_range("conv_alpha_0", self.conv_alpha_0, 0.1, 10.0, "[0.1, 10.0]")?;
        check_range("conv_beta_0", self.conv_beta_0, 0.1, 10.0, "[0.1, 10.0]")?;
        check_unit("conv_lambda", self.conv_lambda)?;
        check_unit("conv_vicarious_discount", self.conv_vicarious_discount)?;
        check_unit("conv_peer_floor", self.conv_peer_floor)?;
        check_range("conv_tau", self.conv_tau, 0.5, 50.0, "[0.5, 50.0]")?;
        check_range("conv_kappa", self.conv_kappa, 0.0, 2.0, "[0.0, 2.0]")?;
        check_range("conv_bias_max", self.conv_bias_max, 0.0, 2.0, "[0.0, 2.0]")?;

        check_u64_range(
            "t_idle_sleep_ms",
            self.t_idle_sleep_ms,
            1_000,
            86_400_000,
            "[1000, 86400000]",
        )?;
        check_usize_range("n_ep_min", self.n_ep_min, 1, 64, "[1, 64]")?;
        check_usize_range("n_ep_max", self.n_ep_max, 4, 1024, "[4, 1024]")?;
        if self.n_ep_min > self.n_ep_max {
            return Err(PriorError::OutOfRange {
                disposition: "n_ep_min",
                value: self.n_ep_min.to_string(),
                range: "[1, n_ep_max]",
            });
        }
        check_range(
            "k_stale_sleep",
            self.k_stale_sleep,
            1.0,
            48.0,
            "[1.0, 48.0]",
        )?;
        check_range(
            "em_boundary_gamma",
            self.em_boundary_gamma,
            0.1,
            5.0,
            "[0.1, 5.0]",
        )?;
        check_usize_range(
            "em_boundary_window",
            self.em_boundary_window,
            5,
            200,
            "[5, 200]",
        )?;
        check_usize_range(
            "em_boundary_warmup",
            self.em_boundary_warmup,
            1,
            50,
            "[1, 50]",
        )?;
        check_unit("em_boundary_mu0", self.em_boundary_mu0)?;
        check_range(
            "em_boundary_sigma0",
            self.em_boundary_sigma0,
            0.01,
            1.0,
            "[0.01, 1.0]",
        )?;
        check_range(
            "em_boundary_m0",
            self.em_boundary_m0,
            0.0,
            50.0,
            "[0.0, 50.0]",
        )?;
        check_range(
            "em_boundary_z_max",
            self.em_boundary_z_max,
            1.0,
            10.0,
            "[1.0, 10.0]",
        )?;
        check_range(
            "em_boundary_sigma_u_floor",
            self.em_boundary_sigma_u_floor,
            0.05,
            2.0,
            "[0.05, 2.0]",
        )?;
        check_usize_range(
            "em_fixed_interval",
            self.em_fixed_interval,
            2,
            256,
            "[2, 256]",
        )?;
        check_range(
            "merge_jaccard_min",
            self.merge_jaccard_min,
            0.5,
            1.0,
            "[0.5, 1.0]",
        )?;
        check_u64_range(
            "merge_gap_ms",
            self.merge_gap_ms,
            0,
            86_400_000,
            "[0, 86400000]",
        )?;

        check_usize_range(
            "domain_clusters_k",
            self.domain_clusters_k,
            1,
            64,
            "[1, 64]",
        )?;
        check_usize_range("domain_m_min", self.domain_m_min, 1, 50, "[1, 50]")?;
        check_usize_range("domain_max_iter", self.domain_max_iter, 5, 100, "[5, 100]")?;
        check_range(
            "domain_w_hist",
            self.domain_w_hist,
            0.0,
            20.0,
            "[0.0, 20.0]",
        )?;
        check_range(
            "domain_tau_match",
            self.domain_tau_match,
            0.30,
            0.99,
            "[0.30, 0.99]",
        )?;
        check_range(
            "domain_tau_rel",
            self.domain_tau_rel,
            0.10,
            0.95,
            "[0.10, 0.95]",
        )?;
        check_range(
            "domain_tau_merge",
            self.domain_tau_merge,
            0.40,
            0.99,
            "[0.40, 0.99]",
        )?;
        check_range(
            "domain_gamma_drift",
            self.domain_gamma_drift,
            0.0,
            2.0,
            "[0.0, 2.0]",
        )?;
        check_range(
            "domain_rho_split",
            self.domain_rho_split,
            0.0,
            0.5,
            "[0.0, 0.5]",
        )?;

        check_usize_range(
            "lesson_min_episodes",
            self.lesson_min_episodes,
            2,
            20,
            "[2, 20]",
        )?;
        check_usize_range(
            "lesson_max_contrasts",
            self.lesson_max_contrasts,
            1,
            10,
            "[1, 10]",
        )?;
        check_range(
            "proc_gamma_beta",
            self.proc_gamma_beta,
            0.50,
            0.999,
            "[0.50, 0.999]",
        )?;
        check_usize_range(
            "proc_cusum_n_min_lock",
            self.proc_cusum_n_min_lock,
            2,
            50,
            "[2, 50]",
        )?;
        check_range(
            "proc_cusum_k_drift",
            self.proc_cusum_k_drift,
            0.01,
            0.50,
            "[0.01, 0.50]",
        )?;
        check_range(
            "proc_cusum_h_warn",
            self.proc_cusum_h_warn,
            0.5,
            10.0,
            "[0.5, 10.0]",
        )?;
        check_range(
            "proc_cusum_h_retire",
            self.proc_cusum_h_retire,
            1.0,
            20.0,
            "[1.0, 20.0]",
        )?;
        check_range(
            "proc_theta_retire",
            self.proc_theta_retire,
            0.05,
            0.70,
            "[0.05, 0.70]",
        )?;
        check_usize_range(
            "ledger_max_items_per_card",
            self.ledger_max_items_per_card,
            4,
            256,
            "[4, 256]",
        )?;
        check_range(
            "self_card_cap_z",
            self.self_card_cap_z,
            0.5,
            4.0,
            "[0.5, 4.0]",
        )?;
        check_usize_range(
            "self_card_cap_min_trials",
            self.self_card_cap_min_trials,
            1,
            50,
            "[1, 50]",
        )?;
        check_range(
            "self_card_cap_deg_threshold",
            self.self_card_cap_deg_threshold,
            0.05,
            0.90,
            "[0.05, 0.90]",
        )?;
        check_u32_range(
            "self_card_cap_consec_fail",
            self.self_card_cap_consec_fail,
            1,
            20,
            "[1, 20]",
        )?;
        check_range(
            "self_card_kappa_bias",
            self.self_card_kappa_bias,
            0.0,
            5.0,
            "[0.0, 5.0]",
        )?;
        check_range(
            "self_card_cal_n0",
            self.self_card_cal_n0,
            0.5,
            50.0,
            "[0.5, 50.0]",
        )?;
        check_range("learn_rate_w", self.learn_rate_w, 0.0, 0.25, "[0.0, 0.25]")?;
        check_range(
            "learn_rate_theta_ratio",
            self.learn_rate_theta_ratio,
            0.0,
            1.0,
            "[0.0, 1.0]",
        )?;
        check_range(
            "learn_daily_max_delta",
            self.learn_daily_max_delta,
            0.0,
            0.50,
            "[0.0, 0.50]",
        )?;
        check_range(
            "learn_ips_clip_m",
            self.learn_ips_clip_m,
            1.0,
            100.0,
            "[1.0, 100.0]",
        )?;
        check_range(
            "learn_wake_gate_n0",
            self.learn_wake_gate_n0,
            0.5,
            50.0,
            "[0.5, 50.0]",
        )?;
        check_range(
            "learn_wake_gate_quantile",
            self.learn_wake_gate_quantile,
            0.01,
            0.50,
            "[0.01, 0.50]",
        )?;

        Ok(())
    }

    /// Maps `talkativeness` in `[0, 1]` to baseline wake threshold `theta_0` (`0.85 -> 0.35`).
    #[must_use]
    pub fn theta_0(&self) -> f64 {
        0.85 - 0.5 * self.talkativeness
    }

    /// Maps `spontaneity` in `[0, 1]` to temperature `tau` (`0.02 -> 0.20`).
    #[must_use]
    pub fn tau(&self) -> f64 {
        0.02 + 0.18 * self.spontaneity
    }

    /// Maps `memory_persistence` in `[0, 1]` to Petrov decay exponent `d` (`0.7 -> 0.3`).
    #[must_use]
    pub fn petrov_d(&self) -> f64 {
        crate::activation::decay_exponent(self.memory_persistence)
    }

    /// Maps `boredom_with_repeats` in `[0, 1]` to habituation decay `gamma` (`0.99 -> 0.80`).
    #[must_use]
    pub fn habituation_gamma(&self) -> f64 {
        0.99 - 0.19 * self.boredom_with_repeats
    }
}

pub(crate) fn encode_prior(w: &mut crate::event::BinWriter, p: &Prior) {
    w.u32(p.version);
    w.str(&p.content_version);
    w.f64(p.talkativeness);
    w.f64(p.spontaneity);
    w.f64(p.memory_persistence);
    w.f64(p.helpfulness);
    w.f64(p.restraint);
    w.f64(p.boredom_with_repeats);
    w.f64(p.certainty_bar);
    w.u8(p.speaking_bar);

    w.f64(p.own_name_weight);
    w.f64(p.info_scale);
    w.f64(p.fok_scale);
    w.f64(p.relevance_weight);
    w.f64(p.commitment_weight);
    w.f64(p.restraint_scale);
    w.f64(p.asr_conf_threshold);
    w.f64(p.asr_clarification_boost);

    w.f64(p.w_b);
    w.f64(p.context_weight);
    w.f64(p.importance_weight);
    w.f64(p.fok_threshold);
    w.f64(p.fok_scale_norm);
    w.f64(p.fade_threshold);

    w.f64(p.kappa);
    w.f64(p.a_scale);
    w.f64(p.budget_weight);
    w.f64(p.promise_debt_weight);
    w.f64(p.reserve_when_unfamiliar);
    w.f64(p.note_band);
    w.f64(p.talk_rate_window_minutes);
    w.f64(p.talk_rate_prior_count);
    w.f64(p.closeness_prior_count);
    w.u64(p.promise_debt_horizon_ms);

    w.f64(p.lambda_c);
    w.f64(p.lambda_a);
    w.f64(p.theta_alloc);
    w.f64(p.theta_ex);
    w.f64(p.m_ex);
    w.f64(p.w_c);
    w.f64(p.w_o);
    w.f64(p.gamma_c);
    w.f64(p.tau_c_s);
    w.u64(p.w_c_window as u64);
    w.f64(p.k_c);
    w.f64(p.beta_fan);
    w.f64(p.theta_trace);

    w.u64(p.patience_place_ms);
    w.u64(p.backlog_max_chunks as u64);

    w.f64(p.step_init);
    w.f64(p.gamma_init);
    w.f64(p.delta_emit);
    w.f64(p.delta_ign);
    w.f64(p.c_base);
    w.f64(p.beta_bp);
    w.f64(p.beta_load);
    w.f64(p.kappa_bp);
    w.f64(p.tau_win_s);
    w.f64(p.lambda_sleep_init);

    w.u64(p.n_min_sleep as u64);
    w.f64(p.alpha_u);
    w.f64(p.tau_wake_hours);
    w.f64(p.d_sleep_max);
    w.f64(p.tau_sw);
    w.f64(p.w_fail);
    w.f64(p.lambda_stakes);
    w.f64(p.theta_sw);
    w.f64(p.w_sw);
    w.f64(p.lambda_fok);
    w.f64(p.lambda_surp);
    w.f64(p.lambda_voi);
    w.f64(p.c0);
    w.f64(p.gamma_opp);
    w.f64(p.alpha_pow);
    w.f64(p.tau_k);
    w.f64(p.w_rec);
    w.f64(p.eta_sl);
    w.f64(p.eta_tr);
    w.f64(p.q_ref);
    w.u64(p.b_steps as u64);
    w.u64(p.n_rec_max as u64);

    w.u64(p.patience_t_min_ms);
    w.u64(p.patience_t_max_ms);
    w.u64(p.patience_default_ms);
    w.f64(p.patience_sigma_ln);
    w.f64(p.patience_prior_n0);
    w.f64(p.patience_place_n0);
    w.f64(p.patience_quantile);
    w.f64(p.patience_decay_rho);

    w.f64(p.map_lambda_decay);
    w.f64(p.map_eta_co);
    w.f64(p.map_w_max);
    w.f64(p.map_gamma_sr);
    w.f64(p.map_alpha_sr);
    w.f64(p.map_tau_step_s);
    w.f64(p.map_ep_gap_s);
    w.f64(p.map_beta_sr);
    w.f64(p.map_delta_ppr);
    w.u64(p.map_hop_budget as u64);
    w.u64(p.map_k_sr as u64);
    w.f64(p.map_w_map);

    w.u64(p.recall_vector_top_k as u64);
    w.u64(p.recall_text_top_k as u64);

    w.u32(p.promise_retry_max);
    w.u64(p.promise_retry_base_ms);
    w.f64(p.agency_tolerance_k);
    w.f64(p.response_expiry_quantile);
    w.f64(p.response_stakes_factor);
    w.f64(p.content_miss_z_threshold);
    w.u64(p.content_miss_min_samples as u64);
    w.f64(p.silence_sample_rate);
    w.f64(p.wake_gate_lo);
    w.f64(p.wake_gate_hi_epsilon);
    w.f64(p.wake_gate_margin);
    w.u64(p.default_goal_expiry_ms);
    w.u64(p.pending_approval_expiry_ms);

    w.u64(p.context_budget_bytes as u64);
    w.u64(p.working_set_full_obs_limit as u64);
    w.u64(p.place_tail_limit as u64);
    w.u64(p.anchor_check_interval_wakes);
    w.u8(p.auto_recall_max);
    w.u8(u8::from(p.tool_rag_enabled));
    w.u64(p.tool_rag_k_cap as u64);
    w.u64(p.tool_rag_k_proc as u64);
    w.u64(p.tool_rag_max_static_core as u64);

    w.f64(p.credence_lambda_domain);
    w.u64(p.credence_period_ms);
    w.u64(p.k_corrob as u64);
    w.f64(p.rule_trust_threshold);
    w.f64(p.conv_alpha_0);
    w.f64(p.conv_beta_0);
    w.f64(p.conv_lambda);
    w.f64(p.conv_vicarious_discount);
    w.f64(p.conv_peer_floor);
    w.f64(p.conv_tau);
    w.f64(p.conv_kappa);
    w.f64(p.conv_bias_max);

    w.u64(p.t_idle_sleep_ms);
    w.u8(u8::from(p.provisional_close));
    w.u64(p.n_ep_min as u64);
    w.u64(p.n_ep_max as u64);
    w.f64(p.k_stale_sleep);
    w.f64(p.em_boundary_gamma);
    w.u64(p.em_boundary_window as u64);
    w.u64(p.em_boundary_warmup as u64);
    w.f64(p.em_boundary_mu0);
    w.f64(p.em_boundary_sigma0);
    w.f64(p.em_boundary_m0);
    w.f64(p.em_boundary_z_max);
    w.f64(p.em_boundary_sigma_u_floor);
    w.u64(p.em_fixed_interval as u64);
    w.f64(p.merge_jaccard_min);
    w.u64(p.merge_gap_ms);

    w.u64(p.domain_clusters_k as u64);
    w.u64(p.domain_m_min as u64);
    w.u64(p.domain_max_iter as u64);
    w.f64(p.domain_w_hist);
    w.f64(p.domain_tau_match);
    w.f64(p.domain_tau_rel);
    w.f64(p.domain_tau_merge);
    w.f64(p.domain_gamma_drift);
    w.f64(p.domain_rho_split);

    w.u64(p.lesson_min_episodes as u64);
    w.u64(p.lesson_max_contrasts as u64);
    w.f64(p.proc_gamma_beta);
    w.u64(p.proc_cusum_n_min_lock as u64);
    w.f64(p.proc_cusum_k_drift);
    w.f64(p.proc_cusum_h_warn);
    w.f64(p.proc_cusum_h_retire);
    w.f64(p.proc_theta_retire);
    w.u8(u8::from(p.allow_chat_habit_bypass));

    w.u64(p.ledger_max_items_per_card as u64);
    w.f64(p.self_card_cap_z);
    w.u64(p.self_card_cap_min_trials as u64);
    w.f64(p.self_card_cap_deg_threshold);
    w.u32(p.self_card_cap_consec_fail);
    w.f64(p.self_card_kappa_bias);
    w.f64(p.self_card_cal_n0);

    w.f64(p.learn_rate_w);
    w.f64(p.learn_rate_theta_ratio);
    w.f64(p.learn_daily_max_delta);
    w.f64(p.learn_ips_clip_m);
    w.f64(p.learn_wake_gate_n0);
    w.f64(p.learn_wake_gate_quantile);
}

fn read_usize(
    r: &mut crate::event::BinReader<'_>,
) -> Result<usize, crate::event::EventFormatError> {
    let raw = r.u64()?;
    usize::try_from(raw).map_err(|_| {
        crate::event::EventFormatError::MalformedRecord(format!("usize overflow {raw}"))
    })
}

pub(crate) fn decode_prior(
    r: &mut crate::event::BinReader<'_>,
) -> Result<Prior, crate::event::EventFormatError> {
    Ok(Prior {
        version: r.u32()?,
        content_version: r.string()?,
        talkativeness: r.f64()?,
        spontaneity: r.f64()?,
        memory_persistence: r.f64()?,
        helpfulness: r.f64()?,
        restraint: r.f64()?,
        boredom_with_repeats: r.f64()?,
        certainty_bar: r.f64()?,
        speaking_bar: r.u8()?,

        own_name_weight: r.f64()?,
        info_scale: r.f64()?,
        fok_scale: r.f64()?,
        relevance_weight: r.f64()?,
        commitment_weight: r.f64()?,
        restraint_scale: r.f64()?,
        asr_conf_threshold: r.f64()?,
        asr_clarification_boost: r.f64()?,

        w_b: r.f64()?,
        context_weight: r.f64()?,
        importance_weight: r.f64()?,
        fok_threshold: r.f64()?,
        fok_scale_norm: r.f64()?,
        fade_threshold: r.f64()?,

        kappa: r.f64()?,
        a_scale: r.f64()?,
        budget_weight: r.f64()?,
        promise_debt_weight: r.f64()?,
        reserve_when_unfamiliar: r.f64()?,
        note_band: r.f64()?,
        talk_rate_window_minutes: r.f64()?,
        talk_rate_prior_count: r.f64()?,
        closeness_prior_count: r.f64()?,
        promise_debt_horizon_ms: r.u64()?,

        lambda_c: r.f64()?,
        lambda_a: r.f64()?,
        theta_alloc: r.f64()?,
        theta_ex: r.f64()?,
        m_ex: r.f64()?,
        w_c: r.f64()?,
        w_o: r.f64()?,
        gamma_c: r.f64()?,
        tau_c_s: r.f64()?,
        w_c_window: read_usize(r)?,
        k_c: r.f64()?,
        beta_fan: r.f64()?,
        theta_trace: r.f64()?,

        patience_place_ms: r.u64()?,
        backlog_max_chunks: read_usize(r)?,

        step_init: r.f64()?,
        gamma_init: r.f64()?,
        delta_emit: r.f64()?,
        delta_ign: r.f64()?,
        c_base: r.f64()?,
        beta_bp: r.f64()?,
        beta_load: r.f64()?,
        kappa_bp: r.f64()?,
        tau_win_s: r.f64()?,
        lambda_sleep_init: r.f64()?,

        n_min_sleep: read_usize(r)?,
        alpha_u: r.f64()?,
        tau_wake_hours: r.f64()?,
        d_sleep_max: r.f64()?,
        tau_sw: r.f64()?,
        w_fail: r.f64()?,
        lambda_stakes: r.f64()?,
        theta_sw: r.f64()?,
        w_sw: r.f64()?,
        lambda_fok: r.f64()?,
        lambda_surp: r.f64()?,
        lambda_voi: r.f64()?,
        c0: r.f64()?,
        gamma_opp: r.f64()?,
        alpha_pow: r.f64()?,
        tau_k: r.f64()?,
        w_rec: r.f64()?,
        eta_sl: r.f64()?,
        eta_tr: r.f64()?,
        q_ref: r.f64()?,
        b_steps: read_usize(r)?,
        n_rec_max: read_usize(r)?,

        patience_t_min_ms: r.u64()?,
        patience_t_max_ms: r.u64()?,
        patience_default_ms: r.u64()?,
        patience_sigma_ln: r.f64()?,
        patience_prior_n0: r.f64()?,
        patience_place_n0: r.f64()?,
        patience_quantile: r.f64()?,
        patience_decay_rho: r.f64()?,

        map_lambda_decay: r.f64()?,
        map_eta_co: r.f64()?,
        map_w_max: r.f64()?,
        map_gamma_sr: r.f64()?,
        map_alpha_sr: r.f64()?,
        map_tau_step_s: r.f64()?,
        map_ep_gap_s: r.f64()?,
        map_beta_sr: r.f64()?,
        map_delta_ppr: r.f64()?,
        map_hop_budget: read_usize(r)?,
        map_k_sr: read_usize(r)?,
        map_w_map: r.f64()?,

        recall_vector_top_k: read_usize(r)?,
        recall_text_top_k: read_usize(r)?,

        promise_retry_max: r.u32()?,
        promise_retry_base_ms: r.u64()?,
        agency_tolerance_k: r.f64()?,
        response_expiry_quantile: r.f64()?,
        response_stakes_factor: r.f64()?,
        content_miss_z_threshold: r.f64()?,
        content_miss_min_samples: read_usize(r)?,
        silence_sample_rate: r.f64()?,
        wake_gate_lo: r.f64()?,
        wake_gate_hi_epsilon: r.f64()?,
        wake_gate_margin: r.f64()?,
        default_goal_expiry_ms: r.u64()?,
        pending_approval_expiry_ms: r.u64()?,

        context_budget_bytes: read_usize(r)?,
        working_set_full_obs_limit: read_usize(r)?,
        place_tail_limit: read_usize(r)?,
        anchor_check_interval_wakes: r.u64()?,
        auto_recall_max: r.u8()?,
        tool_rag_enabled: r.u8()? != 0,
        tool_rag_k_cap: read_usize(r)?,
        tool_rag_k_proc: read_usize(r)?,
        tool_rag_max_static_core: read_usize(r)?,

        credence_lambda_domain: r.f64()?,
        credence_period_ms: r.u64()?,
        k_corrob: read_usize(r)?,
        rule_trust_threshold: r.f64()?,
        conv_alpha_0: r.f64()?,
        conv_beta_0: r.f64()?,
        conv_lambda: r.f64()?,
        conv_vicarious_discount: r.f64()?,
        conv_peer_floor: r.f64()?,
        conv_tau: r.f64()?,
        conv_kappa: r.f64()?,
        conv_bias_max: r.f64()?,

        t_idle_sleep_ms: r.u64()?,
        provisional_close: r.u8()? != 0,
        n_ep_min: read_usize(r)?,
        n_ep_max: read_usize(r)?,
        k_stale_sleep: r.f64()?,
        em_boundary_gamma: r.f64()?,
        em_boundary_window: read_usize(r)?,
        em_boundary_warmup: read_usize(r)?,
        em_boundary_mu0: r.f64()?,
        em_boundary_sigma0: r.f64()?,
        em_boundary_m0: r.f64()?,
        em_boundary_z_max: r.f64()?,
        em_boundary_sigma_u_floor: r.f64()?,
        em_fixed_interval: read_usize(r)?,
        merge_jaccard_min: r.f64()?,
        merge_gap_ms: r.u64()?,

        domain_clusters_k: read_usize(r)?,
        domain_m_min: read_usize(r)?,
        domain_max_iter: read_usize(r)?,
        domain_w_hist: r.f64()?,
        domain_tau_match: r.f64()?,
        domain_tau_rel: r.f64()?,
        domain_tau_merge: r.f64()?,
        domain_gamma_drift: r.f64()?,
        domain_rho_split: r.f64()?,

        lesson_min_episodes: read_usize(r)?,
        lesson_max_contrasts: read_usize(r)?,
        proc_gamma_beta: r.f64()?,
        proc_cusum_n_min_lock: read_usize(r)?,
        proc_cusum_k_drift: r.f64()?,
        proc_cusum_h_warn: r.f64()?,
        proc_cusum_h_retire: r.f64()?,
        proc_theta_retire: r.f64()?,
        allow_chat_habit_bypass: r.u8()? != 0,

        ledger_max_items_per_card: read_usize(r)?,
        self_card_cap_z: r.f64()?,
        self_card_cap_min_trials: read_usize(r)?,
        self_card_cap_deg_threshold: r.f64()?,
        self_card_cap_consec_fail: r.u32()?,
        self_card_kappa_bias: r.f64()?,
        self_card_cal_n0: r.f64()?,

        learn_rate_w: r.f64()?,
        learn_rate_theta_ratio: r.f64()?,
        learn_daily_max_delta: r.f64()?,
        learn_ips_clip_m: r.f64()?,
        learn_wake_gate_n0: r.f64()?,
        learn_wake_gate_quantile: r.f64()?,
    })
}

fn check_unit(disposition: &'static str, value: f64) -> Result<(), PriorError> {
    check_range(disposition, value, 0.0, 1.0, "[0.0, 1.0]")
}

fn check_range(
    disposition: &'static str,
    value: f64,
    min: f64,
    max: f64,
    range: &'static str,
) -> Result<(), PriorError> {
    if !value.is_finite() || value < min || value > max {
        return Err(PriorError::OutOfRange {
            disposition,
            value: value.to_string(),
            range,
        });
    }
    Ok(())
}

fn check_usize_range(
    disposition: &'static str,
    value: usize,
    min: usize,
    max: usize,
    range: &'static str,
) -> Result<(), PriorError> {
    if value < min || value > max {
        return Err(PriorError::OutOfRange {
            disposition,
            value: value.to_string(),
            range,
        });
    }
    Ok(())
}

fn check_u32_range(
    disposition: &'static str,
    value: u32,
    min: u32,
    max: u32,
    range: &'static str,
) -> Result<(), PriorError> {
    if value < min || value > max {
        return Err(PriorError::OutOfRange {
            disposition,
            value: value.to_string(),
            range,
        });
    }
    Ok(())
}

fn check_u64_range(
    disposition: &'static str,
    value: u64,
    min: u64,
    max: u64,
    range: &'static str,
) -> Result<(), PriorError> {
    if value < min || value > max {
        return Err(PriorError::OutOfRange {
            disposition,
            value: value.to_string(),
            range,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Prior, PriorError};

    #[test]
    fn default_prior_is_valid() {
        Prior::default().validate().expect("default prior valid");
    }

    #[test]
    fn minimal_json_deserializes_with_defaults_and_validates() {
        let json = r#"{
            "version": 1,
            "content_version": "2026-11-16.1",
            "talkativeness": 0.5,
            "spontaneity": 0.5,
            "memory_persistence": 0.5,
            "helpfulness": 0.5,
            "restraint": 0.5,
            "boredom_with_repeats": 0.5,
            "certainty_bar": 0.5,
            "speaking_bar": 3
        }"#;
        let parsed: Prior = serde_json::from_str(json).expect("deserialize minimal prior");
        parsed.validate().expect("validate minimal prior");
        assert_eq!(parsed, Prior::default());
    }

    #[test]
    fn out_of_range_disposition_names_field_and_range() {
        let bad = Prior {
            talkativeness: 1.5,
            ..Prior::default()
        };
        let err = bad.validate().expect_err("must reject out of range");
        assert!(matches!(
            err,
            PriorError::OutOfRange {
                disposition: "talkativeness",
                ..
            }
        ));
        let msg = err.to_string();
        assert!(msg.contains("talkativeness"));
        assert!(msg.contains("[0.0, 1.0]"));

        let bad_lambda_c = Prior {
            lambda_c: -0.1,
            ..Prior::default()
        };
        let err2 = bad_lambda_c
            .validate()
            .expect_err("reject negative lambda_c");
        assert!(err2.to_string().contains("lambda_c"));
        assert!(err2.to_string().contains("[0.0, 1.0]"));

        let bad_chunks = Prior {
            backlog_max_chunks: 9,
            ..Prior::default()
        };
        let err3 = bad_chunks.validate().expect_err("reject >6 chunks");
        assert!(err3.to_string().contains("backlog_max_chunks"));
        assert!(err3.to_string().contains("[1, 6]"));
    }

    #[test]
    fn canonical_dispositions_json_matches_prior_default() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prior/dispositions.json");
        let raw = std::fs::read_to_string(&path).expect("read prior/dispositions.json");
        let parsed: Prior =
            serde_json::from_str(&raw).expect("parse prior/dispositions.json cleanly");
        parsed.validate().expect("prior/dispositions.json valid");
        let default = Prior::default();
        if parsed != default {
            let parsed_map = serde_json::to_value(&parsed).expect("serialize parsed");
            let default_map = serde_json::to_value(&default).expect("serialize default");
            let mut diffs = Vec::new();
            if let (Some(pm), Some(dm)) = (parsed_map.as_object(), default_map.as_object()) {
                for (k, pv) in pm {
                    if dm.get(k) != Some(pv) {
                        diffs.push(format!("{k}: file={pv} default={:?}", dm.get(k)));
                    }
                }
            }
            panic!(
                "Prior::default() differs from prior/dispositions.json on disposition(s): {}",
                diffs.join(", ")
            );
        }
    }

    #[test]
    fn prior_rejects_unknown_field_naming_it() {
        let json = r#"{
            "version": 1,
            "content_version": "2026-11-16.1",
            "talkativeness": 0.5,
            "spontaneity": 0.5,
            "memory_persistence": 0.5,
            "helpfulness": 0.5,
            "restraint": 0.5,
            "boredom_with_repeats": 0.5,
            "certainty_bar": 0.5,
            "speaking_bar": 3,
            "tattletale_typo": 0.2
        }"#;
        let err = serde_json::from_str::<Prior>(json).expect_err("must reject unknown field");
        assert!(
            err.to_string().contains("tattletale_typo"),
            "error must name unknown key: {err}"
        );
    }
}
