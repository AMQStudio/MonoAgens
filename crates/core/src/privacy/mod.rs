//! Pre-Step-5 `VisibleScope` projection, `DeclassToken` enforcement, and taint propagation.

pub mod visible_scope;

pub use visible_scope::{
    ScopedCandidate, build_visible_scope, is_valid_declass_token, project_candidate_ids,
    project_visible_scope,
};
