//! Sans-IO cognitive loop turn execution, record-first enforcement, and crash recovery.

pub(crate) mod context_emit;
pub(crate) mod open;
pub(crate) mod sleep_and_beliefs;
pub(crate) mod step;

pub(crate) use open::{open_core, run_startup_recovery, take_snapshot};
pub(crate) use sleep_and_beliefs::{
    apply_gist_verification_verdict, execute_hard_forget_in_core, record_belief_event,
    record_cited_outcome_events, record_convention_event, run_sleep_if_due,
};
pub(crate) use step::execute_turn;
