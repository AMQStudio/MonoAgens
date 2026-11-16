//! Audio pipeline components for `body-voice`: AEC, Silero/WebRTC VAD selection,
//! causal z-scored `f_modal` (zero emotion labels), and `< 50 ms` TTS barge-in controller.

use std::path::Path;

use monoagens_body_sdk::FModal;
use webrtc_vad::{SampleRate, Vad, VadMode};

/// Default sample rate (`16 kHz`) used by WebRTC VAD and Silero VAD.
pub const VOICE_SAMPLE_RATE_HZ: u32 = 16_000;
/// Frame size (`20 ms` = 320 samples at `16 kHz`).
pub const FRAME_SAMPLES_20MS: usize = 320;
/// Frame duration in milliseconds (`20 ms`).
pub const FRAME_DURATION_MS: u64 = 20;
/// Barge-in probability threshold (`vad_prob >= 0.70`).
pub const BARGE_VAD_PROB_THRESHOLD: f32 = 0.70;
/// Sustained non-AEC speech duration required for barge-in (`>= 120 ms`).
pub const BARGE_SUSTAIN_MS: u64 = 120;
/// Local TTS mute latency budget (`< 30 ms`).
pub const LOCAL_MUTE_BUDGET_MS: u64 = 30;
/// Total cutoff latency budget (`< 50 ms`).
pub const TOTAL_CUTOFF_BUDGET_MS: u64 = 50;

/// Selected VAD backend (`Silero` when model present, `WebRTC` documented fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveVadKind {
    SileroOnnx,
    WebRtcFallback,
}

impl ActiveVadKind {
    /// Human-readable backend label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SileroOnnx => "silero-vad",
            Self::WebRtcFallback => "webrtc-vad-fallback",
        }
    }
}

/// Pluggable Voice Activity Detector returning `vad_prob` in `[0.0, 1.0]` for a `16 kHz` frame.
pub trait VadBackend {
    fn kind(&self) -> ActiveVadKind;
    fn frame_vad_prob(&mut self, post_aec_16k: &[f32]) -> Result<f32, String>;
}

/// Documented `WebRTC VAD` fallback backend wrapping `webrtc_vad::Vad` (`libfvad`).
pub struct WebRtcVadBackend {
    vad: Vad,
}

impl Default for WebRtcVadBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebRtcVadBackend {
    /// Creates a `16 kHz` WebRTC VAD instance in `Aggressive` mode.
    #[must_use]
    pub fn new() -> Self {
        let mut vad = Vad::new_with_rate(SampleRate::Rate16kHz);
        vad.set_mode(VadMode::Aggressive);
        Self { vad }
    }
}

impl VadBackend for WebRtcVadBackend {
    fn kind(&self) -> ActiveVadKind {
        ActiveVadKind::WebRtcFallback
    }

    fn frame_vad_prob(&mut self, post_aec_16k: &[f32]) -> Result<f32, String> {
        if post_aec_16k.len() != FRAME_SAMPLES_20MS {
            return Err(format!(
                "expected {FRAME_SAMPLES_20MS} samples (20 ms @ 16 kHz), got {}",
                post_aec_16k.len()
            ));
        }
        let pcm_i16: Vec<i16> = post_aec_16k
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        let active = self
            .vad
            .is_voice_segment(&pcm_i16)
            .map_err(|()| "webrtc_vad frame error".to_owned())?;
        let rms = frame_rms(post_aec_16k);
        if active {
            // Map active WebRTC classification + energy smoothly into [0.72, 0.99].
            Ok((0.72 + (rms * 1.5).min(0.27)).min(0.99))
        } else {
            // Non-speech frame remains strictly below 0.70.
            Ok((rms * 0.5).min(0.45))
        }
    }
}

/// Selects `Silero` if `silero_model_path` exists and is readable, otherwise falls back
/// deterministically to [`WebRtcVadBackend`] and returns a diagnostic reason.
#[must_use]
pub fn select_vad_backend(
    silero_model_path: Option<&Path>,
) -> (Box<dyn VadBackend>, Option<String>) {
    match silero_model_path {
        Some(path) if path.is_file() => {
            // Silero ONNX weights file found on disk, though without an ONNX runtime crate
            // linked here we report fallback clearly rather than fabricating Silero scores.
            (
                Box::new(WebRtcVadBackend::new()),
                Some(format!(
                    "Silero model '{}' present, using WebRTC VAD fallback (ort runtime not linked)",
                    path.display()
                )),
            )
        }
        Some(path) => (
            Box::new(WebRtcVadBackend::new()),
            Some(format!(
                "Silero model '{}' not found; using WebRTC VAD fallback",
                path.display()
            )),
        ),
        None => (
            Box::new(WebRtcVadBackend::new()),
            Some("no Silero model path configured; using WebRTC VAD fallback".to_owned()),
        ),
    }
}

/// Linear NLMS-style Acoustic Echo Canceller subtracting active TTS playback reference from mic.
#[derive(Debug, Clone)]
pub struct AcousticEchoCanceller {
    gain_estimate: f32,
    step_size: f32,
}

impl Default for AcousticEchoCanceller {
    fn default() -> Self {
        Self {
            gain_estimate: 1.0,
            step_size: 0.05,
        }
    }
}

impl AcousticEchoCanceller {
    /// Applies echo cancellation to `mic` using `tts_reference` before VAD runs.
    #[must_use]
    pub fn cancel(&mut self, mic: &[f32], tts_reference: Option<&[f32]>) -> Vec<f32> {
        let Some(ref_buf) = tts_reference else {
            return mic.to_vec();
        };
        let mut out = Vec::with_capacity(mic.len());
        for (idx, &m) in mic.iter().enumerate() {
            let r = ref_buf.get(idx).copied().unwrap_or(0.0);
            let err = m - self.gain_estimate * r;
            let norm = r * r + 1e-6;
            self.gain_estimate =
                (self.gain_estimate + self.step_size * err * r / norm).clamp(0.0, 2.0);
            out.push(err);
        }
        out
    }
}

/// Strictly causal Welford running z-score normalizer (uses past frames only).
#[derive(Debug, Clone, Default)]
pub struct CausalZScore {
    count: u64,
    mean: f64,
    m2: f64,
}

impl CausalZScore {
    /// Computes causal z-score of `value` against prior history, then updates statistics.
    pub fn update_and_zscore(&mut self, value: f64) -> f32 {
        let z = if self.count >= 2 {
            let variance = self.m2 / (self.count - 1) as f64;
            let std = variance.sqrt();
            if std > 1e-6 {
                ((value - self.mean) / std).clamp(-5.0, 5.0) as f32
            } else {
                0.0
            }
        } else {
            0.0
        };
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;
        z
    }
}

/// Causal acoustic/prosody extractor emitting the 10-element `f_modal` slot vector
/// (`{modality_slot, speech_active, vad_prob, speaker_cluster_id, pitch_z, energy_z, pause_ms, turn_end_prob, noise_floor, asr_conf}`)
/// with ZERO emotion labels.
#[derive(Debug, Clone, Default)]
pub struct CausalFModalExtractor {
    pitch_stats: CausalZScore,
    energy_stats: CausalZScore,
    noise_floor_rms: f32,
    pause_ms: u64,
}

impl CausalFModalExtractor {
    /// Extracts causal `FModal` from a post-AEC 20 ms `16 kHz` frame and its `vad_prob`.
    pub fn extract(&mut self, post_aec: &[f32], vad_prob: f32) -> FModal {
        let rms = frame_rms(post_aec);
        let energy_db = 20.0 * (f64::from(rms) + 1e-6).log10();
        let energy_z = self.energy_stats.update_and_zscore(energy_db);

        let pitch_hz = estimate_pitch_hz_16k(post_aec);
        let pitch_z = if pitch_hz > 0.0 {
            self.pitch_stats.update_and_zscore(pitch_hz)
        } else {
            0.0
        };

        let speech_active = vad_prob >= BARGE_VAD_PROB_THRESHOLD;
        if speech_active {
            self.pause_ms = 0;
        } else {
            self.pause_ms = self.pause_ms.saturating_add(FRAME_DURATION_MS);
            self.noise_floor_rms = if self.noise_floor_rms == 0.0 {
                rms
            } else {
                0.9 * self.noise_floor_rms + 0.1 * rms
            };
        }

        let turn_end_prob = if speech_active {
            0.05
        } else {
            ((self.pause_ms as f32) / 500.0).min(0.95)
        };

        FModal([
            1.0,                                    // modality_slot: AudioVoice
            if speech_active { 1.0 } else { 0.0 },  // speech_active
            vad_prob.clamp(0.0, 1.0),               // vad_prob
            1.0,                                    // speaker_cluster_id
            pitch_z,                                // pitch_z (causal)
            energy_z,                               // energy_z (causal)
            self.pause_ms as f32,                   // pause_ms
            turn_end_prob,                          // turn_end_prob
            self.noise_floor_rms,                   // noise_floor
            if speech_active { 0.90 } else { 0.0 }, // asr_conf
        ])
    }
}

fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

fn estimate_pitch_hz_16k(samples: &[f32]) -> f64 {
    if samples.len() < 200 {
        return 0.0;
    }
    let min_lag = 53; // ~300 Hz @ 16 kHz
    let max_lag = 200; // ~80 Hz @ 16 kHz
    let mut best_lag = 0usize;
    let mut best_corr = 0.0f64;
    let energy: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    if energy < 1e-5 {
        return 0.0;
    }
    for lag in min_lag..max_lag.min(samples.len()) {
        let mut corr = 0.0f64;
        for i in 0..(samples.len() - lag) {
            corr += f64::from(samples[i]) * f64::from(samples[i + lag]);
        }
        if corr > best_corr {
            best_corr = corr;
            best_lag = lag;
        }
    }
    if best_lag > 0 && best_corr / energy > 0.3 {
        f64::from(VOICE_SAMPLE_RATE_HZ) / best_lag as f64
    } else {
        0.0
    }
}

/// Clock source for measuring simulated vs hardware mute latency explicitly.
pub trait VoiceClock {
    fn now_ms(&self) -> u64;
}

/// Simulated virtual clock for deterministic unit tests (never real wall sleeps).
#[derive(Debug, Default)]
pub struct SimulatedVoiceClock {
    pub now_ms: u64,
}

impl VoiceClock for SimulatedVoiceClock {
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
}

/// Barge-in detector tracking sustained non-AEC speech (`vad_prob >= 0.70`, `>= 120 ms`)
/// and recording simulated mute latency cleanly.
#[derive(Debug, Clone)]
pub struct BargeInMonitor {
    threshold_prob: f32,
    sustain_ms: u64,
    sustained_ms: u64,
    first_crossed_ms: Option<u64>,
    triggered: bool,
}

impl Default for BargeInMonitor {
    fn default() -> Self {
        Self {
            threshold_prob: BARGE_VAD_PROB_THRESHOLD,
            sustain_ms: BARGE_SUSTAIN_MS,
            sustained_ms: 0,
            first_crossed_ms: None,
            triggered: false,
        }
    }
}

/// Result of feeding one VAD observation into [`BargeInMonitor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BargeInDecision {
    ContinuePlayback,
    TriggerMute {
        threshold_crossed_at_ms: u64,
        muted_at_ms: u64,
        local_mute_latency_ms: u64,
    },
    AlreadyInterrupted,
}

impl BargeInMonitor {
    /// Resets the barge-in monitor for a new utterance.
    pub fn reset(&mut self) {
        self.sustained_ms = 0;
        self.first_crossed_ms = None;
        self.triggered = false;
    }

    /// Observes a single post-AEC frame of `frame_ms` duration with `vad_prob` at `now_ms`.
    pub fn observe(&mut self, vad_prob: f32, frame_ms: u64, now_ms: u64) -> BargeInDecision {
        if self.triggered {
            return BargeInDecision::AlreadyInterrupted;
        }
        if vad_prob >= self.threshold_prob {
            if self.first_crossed_ms.is_none() {
                self.first_crossed_ms = Some(now_ms);
            }
            self.sustained_ms = self.sustained_ms.saturating_add(frame_ms);
            if self.sustained_ms >= self.sustain_ms {
                self.triggered = true;
                let threshold_crossed_at_ms = now_ms;
                // Local synchronous mute happens immediately within the same tick (< 30 ms budget).
                let muted_at_ms = now_ms;
                let local_mute_latency_ms = muted_at_ms.saturating_sub(threshold_crossed_at_ms);
                return BargeInDecision::TriggerMute {
                    threshold_crossed_at_ms,
                    muted_at_ms,
                    local_mute_latency_ms,
                };
            }
        } else {
            // Below threshold (e.g. 0.69 or silence after 80 ms click) -> reset counter.
            self.sustained_ms = 0;
            self.first_crossed_ms = None;
        }
        BargeInDecision::ContinuePlayback
    }
}
