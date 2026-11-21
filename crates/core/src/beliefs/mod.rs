//! Beta-Bernoulli belief credences, source reliability, corroboration, and conventions.

pub mod credence;

pub use credence::{
    BeliefBook, BeliefLogEntry, BeliefRecord, BetaCredence, DEFAULT_DOMAIN_ID, parse_basis_str,
    verify_claimed_basis,
};
