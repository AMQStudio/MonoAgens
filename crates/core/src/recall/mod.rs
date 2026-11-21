//! Memory activation, hybrid recall, dual-process familiarity/recollection, importance,
//! and pre-Step-5 `VisibleScope` projection.

pub mod engine;
pub mod importance;
pub mod recollection;

pub use crate::privacy::visible_scope;
pub use engine::{
    DEFAULT_TEXT_TOP_K, DEFAULT_VECTOR_TOP_K, RecallQuery, RecallResult, RecalledItem, run_recall,
    run_recall_with_model,
};
pub use importance::{ImportanceInputs, compute_importance};
pub use recollection::{
    ContextMatchTag, MemoryBasis, MemoryEntry, RecollectionConstraint, SourceEventFact,
    classify_context_match, evaluate_familiarity, verify_recollection,
};
