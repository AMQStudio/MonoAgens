use monoagens_body_kit::{
    BargeInToken, BubbleSender, EchoRegistry, HonestCueKind, HonestCueThresholds, HonestCueTracker,
    PersistentBodyRuntime, deliver_split_bubbles, split_text_chunks,
};
use monoagens_body_sdk::{ActionId, CapabilityId, ReceiptOutcome, TerminationReason, Time};
use tempfile::TempDir;

struct RecordingSender {
    sent: Vec<(usize, String, String)>,
    barge_after_count: Option<usize>,
    barge_token: BargeInToken,
}

impl BubbleSender for RecordingSender {
    fn send_bubble(&mut self, chunk_index: usize, chunk_text: &str) -> Result<String, String> {
        let pid = format!("pid_{}", chunk_index + 1);
        self.sent
            .push((chunk_index, chunk_text.to_owned(), pid.clone()));
        if self
            .barge_after_count
            .is_some_and(|limit| self.sent.len() >= limit)
        {
            self.barge_token.trigger_external();
        }
        Ok(pid)
    }
}

#[test]
fn three_bubble_output_aborted_after_pid_1() {
    let text = "bubble one!! bubble two!! bubble three";
    let chunks = split_text_chunks(text, 14).expect("split chunks");
    assert_eq!(chunks.len(), 3, "expected 3 chunks, got {chunks:?}");
    assert!(chunks.iter().all(|c| c.len() <= 14));

    let barge_in = BargeInToken::new();
    let mut sender = RecordingSender {
        sent: Vec::new(),
        barge_after_count: Some(1),
        barge_token: barge_in.clone(),
    };
    let mut echo = EchoRegistry::default();

    let outcome = deliver_split_bubbles(
        ActionId::new("act-split-3"),
        CapabilityId::new("chat.speak"),
        text,
        14,
        &mut sender,
        &barge_in,
        &mut echo,
        None,
        Time::from_millis(5_000),
    )
    .expect("deliver");

    assert_eq!(outcome.total_chunks, 3);
    assert_eq!(sender.sent.len(), 1, "chunks 2 and 3 must never be sent");
    assert_eq!(outcome.delivered_pids, vec!["pid_1"]);
    assert_eq!(outcome.receipt.outcome, ReceiptOutcome::Failed);
    assert_eq!(
        outcome.receipt.termination_reason,
        Some(TerminationReason::PreemptedExternal)
    );
    assert_eq!(outcome.receipt.delivered_units, Some(1));
    let progress = outcome.receipt.progress.expect("progress");
    assert!((progress - (1.0 / 3.0)).abs() < 1e-4, "progress={progress}");

    assert!(echo.is_self_echo("pid_1"));
    assert!(!echo.is_self_echo("pid_2"));
    assert!(!echo.is_self_echo("pid_3"));
    let resolved = echo.resolve_reply_to("pid_1").expect("reply-to pid_1");
    assert_eq!(resolved.action_id, ActionId::new("act-split-3"));
    assert_eq!(resolved.chunk_index, 0);
}

#[test]
fn full_delivery_of_m_bubbles_reports_delivered_units_m() {
    let text = "first chunk. second chunk. third chunk. fourth";
    let chunks = split_text_chunks(text, 14).expect("split");
    let m = chunks.len();
    assert!(m >= 3, "expected >= 3 chunks, got {chunks:?}");

    let barge_in = BargeInToken::new();
    let mut sender = RecordingSender {
        sent: Vec::new(),
        barge_after_count: None,
        barge_token: barge_in.clone(),
    };
    let mut echo = EchoRegistry::default();

    let outcome = deliver_split_bubbles(
        ActionId::new("act-full"),
        CapabilityId::new("chat.speak"),
        text,
        14,
        &mut sender,
        &barge_in,
        &mut echo,
        None,
        Time::from_millis(6_000),
    )
    .expect("deliver");

    assert_eq!(sender.sent.len(), m);
    assert_eq!(outcome.delivered_pids.len(), m);
    assert_eq!(outcome.receipt.outcome, ReceiptOutcome::Done);
    assert_eq!(
        outcome.receipt.termination_reason,
        Some(TerminationReason::Normal)
    );
    assert_eq!(outcome.receipt.delivered_units, Some(m as u64));
    assert_eq!(outcome.receipt.progress, Some(1.0));
    for pid in &outcome.delivered_pids {
        assert!(echo.is_self_echo(pid));
    }
}

#[test]
fn barge_in_before_any_chunk_sends_nothing_and_reports_progress_zero() {
    let text = "alpha beta gamma delta epsilon";
    let barge_in = BargeInToken::new();
    barge_in.trigger_external();

    let mut sender = RecordingSender {
        sent: Vec::new(),
        barge_after_count: None,
        barge_token: barge_in.clone(),
    };
    let mut echo = EchoRegistry::default();

    let outcome = deliver_split_bubbles(
        ActionId::new("act-preempt-0"),
        CapabilityId::new("chat.speak"),
        text,
        10,
        &mut sender,
        &barge_in,
        &mut echo,
        None,
        Time::from_millis(7_000),
    )
    .expect("deliver");

    assert!(sender.sent.is_empty());
    assert!(outcome.delivered_pids.is_empty());
    assert_eq!(outcome.receipt.outcome, ReceiptOutcome::Failed);
    assert_eq!(outcome.receipt.delivered_units, Some(0));
    assert_eq!(outcome.receipt.progress, Some(0.0));
    assert!(echo.is_empty());
}

#[test]
fn honest_cue_tracker_emits_only_past_thresholds() {
    let mut tracker = HonestCueTracker::start("wait-1", 10_000, HonestCueThresholds::default());
    assert_eq!(tracker.thresholds().d_art_ms, 0);
    assert!(tracker.poll(10_000).is_empty());
    assert!(tracker.poll(10_999).is_empty());

    let at_1s = tracker.poll(11_000);
    assert_eq!(at_1s.len(), 1);
    assert_eq!(at_1s[0].kind, HonestCueKind::WorkingCue);
    assert_eq!(at_1s[0].elapsed_ms, 1_000);

    assert!(tracker.poll(19_999).is_empty());

    let at_10s = tracker.poll(20_000);
    assert_eq!(at_10s.len(), 1);
    assert_eq!(at_10s[0].kind, HonestCueKind::ProgressNote);
    assert_eq!(at_10s[0].elapsed_ms, 10_000);

    // Never duplicates after emission.
    assert!(tracker.poll(30_000).is_empty());

    // Fast completion emits zero cues.
    let mut fast = HonestCueTracker::start("wait-fast", 1_000, HonestCueThresholds::default());
    fast.finish();
    assert!(fast.poll(50_000).is_empty());
}

#[test]
fn persistent_runtime_survives_drop_and_reload() {
    let tmp = TempDir::new().expect("tempdir");
    let receipt = monoagens_body_kit::make_terminal_receipt(
        ActionId::new("act-persist-1"),
        CapabilityId::new("chat.speak"),
        ReceiptOutcome::Done,
        None,
        Some("ok".to_owned()),
        Some(TerminationReason::Normal),
        Some(1.0),
        Some(1),
        None,
        Time::from_millis(123),
    );
    {
        let mut rt = PersistentBodyRuntime::new(Some(tmp.path().to_path_buf()));
        rt.record_terminal(&receipt);
    }
    let reloaded = PersistentBodyRuntime::new(Some(tmp.path().to_path_buf()));
    assert_eq!(
        reloaded.journal_receipt(&ActionId::new("act-persist-1")),
        Some(&receipt)
    );
}
