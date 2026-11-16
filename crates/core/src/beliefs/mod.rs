//! Beta-Bernoulli belief credences, source reliability, corroboration, conventions,
//! and Hungarian cross-snapshot domain alignment (`§7`, `A-01`, `A-12`, `R-17`, `R-32`, `R-34`, `R-43`).

pub mod credence;
pub mod domains;

pub use credence::{
    BeliefBook, BeliefLogEntry, BeliefRecord, BetaCredence, parse_basis_str, verify_claimed_basis,
};
pub use domains::{
    CredenceRebaseRecord, DEFAULT_DOMAIN_ID, DomainCentroid, DomainClusterBook,
    DomainClusterUpdate, DomainTransitionKind, DomainTransitionRecord, hungarian_min_cost,
};
