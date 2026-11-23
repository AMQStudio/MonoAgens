use std::f32::consts::PI;

use body_voice::{
    ActiveVadKind, FRAME_DURATION_MS, FRAME_SAMPLES_20MS, LOCAL_MUTE_BUDGET_MS,
    TOTAL_CUTOFF_BUDGET_MS, VOICE_SAMPLE_RATE_HZ, VoiceBody,
};
use monoagens_body_sdk::{ActionId, ReceiptOutcome, TerminationReason};

fn voiced_sine_frame(freq_hz: f32, amp: f32) -> Vec<f32> {
    (0..FRAME_SAMPLES_20MS)
        .map(|i| amp * (2.0 * PI * freq_hz * (i as f32) / (VOICE_SAMPLE_RATE_HZ as f32)).sin())
        .collect()
}

#[test]
fn voice_80ms_click_continues_playback_while_200ms_speech_preempts_at_42_percent() {
    let mut body = VoiceBody::with_memory_sink(None);
    assert_eq!(body.active_vad_kind(), ActiveVadKind::WebRtcFallback);
    assert!(body.vad_fallback_note().is_some());

    // 1. 80 ms backchannel click (4 x 20 ms frames >= 0.70, then silence) keeps playback!
    body.start_streaming_speech(ActionId::new("act-voice-1"), "hello world", 10, None);
    body.set_playback_progress(0.20, 2);

    let click_frame = voiced_sine_frame(180.0, 0.35);
    let silence_frame = vec![0.0f32; FRAME_SAMPLES_20MS];
    for i in 0..4 {
        let (percept, receipt) = body
            .process_vad_observation(
                &click_frame,
                0.85,
                FRAME_DURATION_MS,
                1_000 + i * FRAME_DURATION_MS,
                "simulated_clock",
                format!("k-click-{i}"),
            )
            .expect("frame");
        assert!(receipt.is_none(), "80 ms click must NOT preempt");
        assert!(percept.f_modal.is_some());
    }
    // Silence resets sustained window.
    let (_p, r_sil) = body
        .process_vad_observation(
            &silence_frame,
            0.05,
            FRAME_DURATION_MS,
            1_080,
            "simulated_clock",
            "k-sil",
        )
        .expect("silence");
    assert!(r_sil.is_none());
    assert!(!body.tts_sink().muted);

    // 2. Now user speaks for 200 ms (10 x 20 ms frames) while playback is at 42% (delivered_units=4).
    body.set_playback_progress(0.42, 4);
    let mut receipts = Vec::new();
    for i in 0..10 {
        let (_p, r) = body
            .process_vad_observation(
                &click_frame,
                0.88,
                FRAME_DURATION_MS,
                2_000 + i * FRAME_DURATION_MS,
                "simulated_clock",
                format!("k-barge-{i}"),
            )
            .expect("frame");
        if let Some(rcpt) = r {
            receipts.push(rcpt);
        }
    }

    assert_eq!(
        receipts.len(),
        1,
        "exactly ONE terminal receipt on barge-in"
    );
    let rcpt = &receipts[0];
    assert_eq!(rcpt.outcome, ReceiptOutcome::Failed);
    assert_eq!(
        rcpt.termination_reason,
        Some(TerminationReason::PreemptedExternal)
    );
    assert_eq!(rcpt.progress, Some(0.42));
    assert_eq!(rcpt.delivered_units, Some(4));
    assert!(body.tts_sink().muted);

    let m = body.last_mute_measurement().expect("mute measurement");
    assert_eq!(m.clock_kind, "simulated_clock");
    assert!(m.local_mute_latency_ms < LOCAL_MUTE_BUDGET_MS);
    assert!(m.total_cutoff_latency_ms < TOTAL_CUTOFF_BUDGET_MS);

    // Non-interrupted finish after preemption emits zero duplicate receipts.
    assert!(body.finish_streaming_speech(2_500).is_none());
}

#[test]
fn voice_threshold_edges_069_vs_070_and_119ms_vs_120ms() {
    let mut body = VoiceBody::with_memory_sink(None);
    let frame = voiced_sine_frame(160.0, 0.25);

    // Edge A: vad_prob = 0.69 sustained for 200 ms never triggers.
    body.start_streaming_speech(ActionId::new("act-edge-prob"), "edge test", 5, None);
    for i in 0..10 {
        let (_p, r) = body
            .process_vad_observation(
                &frame,
                0.69,
                FRAME_DURATION_MS,
                i * FRAME_DURATION_MS,
                "simulated_clock",
                format!("p69-{i}"),
            )
            .expect("frame");
        assert!(r.is_none());
    }
    assert!(!body.tts_sink().muted);

    // Edge B: vad_prob = 0.70 for 119 ms does NOT trigger; +1 ms (reaching 120 ms) triggers!
    let (_p, r_119) = body
        .process_vad_observation(&frame, 0.70, 119, 5_000, "simulated_clock", "d119")
        .expect("119ms");
    assert!(r_119.is_none());
    assert!(!body.tts_sink().muted);

    let (_p, r_120) = body
        .process_vad_observation(&frame, 0.70, 1, 5_001, "simulated_clock", "d120")
        .expect("120ms");
    let rcpt = r_120.expect("120ms must trigger preemption receipt");
    assert_eq!(
        rcpt.termination_reason,
        Some(TerminationReason::PreemptedExternal)
    );
    assert!(body.tts_sink().muted);
}

#[test]
fn voice_aec_suppresses_tts_self_echo_before_webrtc_vad_and_emits_causal_f_modal() {
    let mut body = VoiceBody::with_memory_sink(None);
    body.start_streaming_speech(ActionId::new("act-aec"), "tts self output", 4, None);

    // Pure TTS echo in mic matching tts_ref -> AEC zeroes it out -> WebRTC VAD sees silence!
    let tts_ref = voiced_sine_frame(200.0, 0.5);
    for i in 0..8 {
        let (percept, rcpt) = body
            .process_audio_frame(
                &tts_ref,
                Some(&tts_ref),
                10_000 + i * FRAME_DURATION_MS,
                "simulated_clock",
                format!("aec-{i}"),
            )
            .expect("aec frame");
        assert!(rcpt.is_none(), "AEC must prevent self-echo barge-in");
        let f_modal = percept.f_modal.expect("f_modal present");
        assert_eq!(f_modal.0[0], 1.0); // AudioVoice slot
        assert!(f_modal.0[2] < 0.70); // vad_prob below threshold after AEC
    }
    assert!(!body.tts_sink().muted);
}
