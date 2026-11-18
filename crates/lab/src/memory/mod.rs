//! Memory-comparison suite: metering proxy, judge, scoring, phase 0 runner and memory report.

pub mod judge;
pub mod prompts;
pub mod proxy;
pub mod report;
pub mod scoring;
pub mod suite;

pub use judge::{HttpJudgeClient, JudgeRunner};
pub use prompts::{
    ANSWER_FORMAT_PROMPT_VERSION, LOCOMO_ACCURACY_PROMPT, LOCOMO_ANSWER_FORMAT_V1,
    LOCOMO_PROMPT_VERSION, LONGMEMEVAL_ANSWER_FORMAT_V1, LONGMEMEVAL_PROMPT_VERSION,
    locomo_judge_prompt, longmemeval_judge_prompt, parse_locomo_verdict, parse_longmemeval_verdict,
};
pub use proxy::{
    CallAttribution, LedgerEntry, LedgerTotals, MeteringProxy, MeteringProxyConfig,
    OPENCODE_GO_CHAT_COMPLETIONS_URL, PRICE_CACHED_INPUT_PER_M, PRICE_OUTPUT_PER_M,
    PRICE_UNCACHED_INPUT_PER_M, list_cost_usd, parse_usage, peak_multiplier_at, read_ledger_totals,
    session_id_for_attribution,
};
pub use report::{GeneratedMemoryReport, generate_memory_report};
pub use scoring::{MemoryItemScore, MemoryScore, score_locomo, score_longmemeval};
pub use suite::{
    DatasetArtifact, MemoryPhase, MemoryRunFile, MemorySuiteOptions, MemorySuiteOutcome,
    PilotSummary, run_memory_suite, select_pilot_locomo_ids, select_pilot_longmemeval_ids,
};
