//! Pre-Step-5 `VisibleScope` projection (`R-33`, `Δ_NI = 0`).
//!
//! All candidate memories, activations, and expectations MUST pass through this
//! projection BEFORE Step-4 `feeling_of_knowing`, `relevance`, recall ranking,
//! or use-count updates. Rejected items return nothing: no counts, no titles,
//! and no activation updates.

use monoagens_protocol::{
    ActorId, Audience, DeclassToken, EventId, OwnerSigned, PlaceId, Time, VisibleScope,
};

/// Trait implemented by any candidate record carrying an [`Audience`] and optional [`EventId`].
pub trait ScopedCandidate {
    fn item_audience(&self) -> &Audience;
    fn item_event_id(&self) -> Option<EventId>;
}

/// Projects `candidates` through `scope` BEFORE scoring, returning only permitted references.
#[must_use]
pub fn project_visible_scope<'a, T: ScopedCandidate>(
    scope: &VisibleScope,
    candidates: &'a [T],
    now: Time,
) -> Vec<&'a T> {
    candidates
        .iter()
        .filter(|item| scope.allows(item.item_audience(), item.item_event_id(), now))
        .collect()
}

/// Projects candidate `(EventId, Audience)` pairs through `scope` BEFORE scoring, top-k,
/// or use-count updates, returning only permitted `EventId`s with zero rejected metadata.
#[must_use]
pub fn project_candidate_ids<'a, F>(
    scope: &VisibleScope,
    candidate_ids: impl IntoIterator<Item = EventId>,
    mut resolve_audience: F,
    now: Time,
) -> Vec<EventId>
where
    F: FnMut(EventId) -> Option<&'a Audience>,
{
    let mut out = Vec::new();
    for id in candidate_ids {
        if let Some(aud) = resolve_audience(id)
            && scope.allows(aud, Some(id), now)
        {
            out.push(id);
        }
    }
    out
}

/// Validates that an owner-signed [`DeclassToken`] is structurally signed, matches the
/// anchored owner signer (if present), and is not expired at `now`.
#[must_use]
pub fn is_valid_declass_token(
    signed: &OwnerSigned<DeclassToken>,
    expected_owner: Option<&ActorId>,
    now: Time,
) -> bool {
    if signed.signature.trim().is_empty() || signed.payload.signature.trim().is_empty() {
        return false;
    }
    if signed.payload.issuer != signed.signer {
        return false;
    }
    if let Some(owner) = expected_owner
        && &signed.signer != owner
    {
        return false;
    }
    if signed.payload.token_id.trim().is_empty() || signed.payload.scope.is_empty() {
        return false;
    }
    now <= signed.payload.expiry
}

/// Constructs a [`VisibleScope`] for `(audience, target_place)` populated solely with
/// active, valid, owner-signed [`DeclassToken`]s logged in Core.
#[must_use]
pub fn build_visible_scope(
    audience: Audience,
    target_place: Option<PlaceId>,
    logged_tokens: &[OwnerSigned<DeclassToken>],
    expected_owner: Option<&ActorId>,
    now: Time,
) -> VisibleScope {
    let mut scope = VisibleScope::new(audience, target_place);
    for signed in logged_tokens {
        if is_valid_declass_token(signed, expected_owner, now) {
            scope.declass_tokens.push(signed.payload.clone());
        }
    }
    scope
}
