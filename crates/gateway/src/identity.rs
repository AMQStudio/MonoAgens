//! Canonical cross-body `κ(b, a)` identity resolution and append-only bilateral
//! dual-nonce account linking (`4.3`, `R-44`).
//!
//! Resolution precedence (strictly deterministic, zero heuristic merging):
//! 1. Configured `[[person]]` owner/static mapping (`C_person`)
//! 2. Active non-revoked append-only link (`E_link`: `actor_linked` / `actor_unlinked`)
//! 3. Length-prefixed `per_<hex32(BLAKE3("monoagens.actor.v1" || κ(b, a)))>`

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use monoagens_protocol::{ActorId, BodyId, PrincipalLink, PrincipalLinkStatus, Time};
use serde::{Deserialize, Serialize};

use crate::config::{GatewayConfig, IdentityConfig, PersonMappingConfig};
use crate::seams::IdentitySeam;

/// Domain separation prefix for canonical `per_` actor ID derivation.
pub const ACTOR_ID_DOMAIN_PREFIX: &[u8] = b"monoagens.actor.v1";

/// Domain separation prefix for bilateral link nonce commitments.
pub const LINK_NONCE_DOMAIN_PREFIX: &[u8] = b"monoagens.link.nonce.v1";

/// Heuristic cross-body merging is permanently disabled (`T_μ = +∞`, `R-44`).
pub const HEURISTIC_MERGE_ENABLED: bool = false;

/// Fellegi-Sunter merge threshold (`+∞`, `R-44`).
pub const HEURISTIC_MERGE_THRESHOLD_MU: f64 = f64::INFINITY;

/// Default bilateral challenge expiry (`600 s`).
pub const DEFAULT_LINK_CHALLENGE_TTL_S: u64 = 600;

/// Default maximum verification attempts per bilateral challenge (`3`).
pub const DEFAULT_LINK_MAX_ATTEMPTS: u32 = 3;

/// Default owner escalation guard (`true`, invariant-locked).
pub const OWNER_ESCALATION_GUARD: bool = true;

/// Encodes length-prefixed canonical key `κ(b, a) = u32_le(|b|) || b || u32_le(|a|) || a`.
///
/// Eliminates delimiter/colon collisions across Matrix MXIDs, DIDs, JIDs, dots, unicode,
/// and empty components.
#[must_use]
pub fn canonical_kappa(body_id: &str, account_id: &str) -> Vec<u8> {
    let b = body_id.as_bytes();
    let a = account_id.as_bytes();
    let b_len = u32::try_from(b.len()).unwrap_or(u32::MAX);
    let a_len = u32::try_from(a.len()).unwrap_or(u32::MAX);

    let mut out = Vec::with_capacity(8 + b.len() + a.len());
    out.extend_from_slice(&b_len.to_le_bytes());
    out.extend_from_slice(b);
    out.extend_from_slice(&a_len.to_le_bytes());
    out.extend_from_slice(a);
    out
}

/// Derives deterministic `per_<hex32(BLAKE3("monoagens.actor.v1" || κ(b, a)))>`.
#[must_use]
pub fn derive_per_actor_id(body_id: &BodyId, account_id: &str) -> ActorId {
    let kappa = canonical_kappa(body_id.as_str(), account_id);
    let mut hasher = blake3::Hasher::new();
    hasher.update(ACTOR_ID_DOMAIN_PREFIX);
    hasher.update(&kappa);
    let hex = hasher.finalize().to_hex();
    ActorId::new(format!("per_{}", &hex[..32]))
}

/// Errors produced by identity resolution or bilateral challenge linking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    Io {
        path: String,
        detail: String,
    },
    Parse {
        detail: String,
    },
    EntropyFailure(String),
    InvariantViolation(&'static str),
    ChallengeNotFound {
        challenge_id: String,
    },
    ChallengeAlreadySettled {
        challenge_id: String,
    },
    ChallengeExpired {
        challenge_id: String,
        expires_at_ms: Time,
        now_ms: Time,
    },
    MaxAttemptsExceeded {
        challenge_id: String,
        max_attempts: u32,
    },
    NonceMismatch {
        challenge_id: String,
    },
    OwnerEscalationDenied {
        target_actor: ActorId,
        secondary_body_id: BodyId,
        secondary_account_id: String,
    },
    ConfigTakesPrecedence {
        body_id: BodyId,
        account_id: String,
        configured_actor: ActorId,
    },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, detail } => write!(f, "identity log I/O error at '{path}': {detail}"),
            Self::Parse { detail } => write!(f, "identity log parse error: {detail}"),
            Self::EntropyFailure(detail) => write!(f, "CSPRNG failure generating nonce: {detail}"),
            Self::InvariantViolation(msg) => write!(f, "identity invariant violation: {msg}"),
            Self::ChallengeNotFound { challenge_id } => {
                write!(f, "link challenge '{challenge_id}' not found")
            }
            Self::ChallengeAlreadySettled { challenge_id } => {
                write!(f, "link challenge '{challenge_id}' already completed")
            }
            Self::ChallengeExpired {
                challenge_id,
                expires_at_ms,
                now_ms,
            } => write!(
                f,
                "link challenge '{challenge_id}' expired at {expires_at_ms} (now {now_ms})"
            ),
            Self::MaxAttemptsExceeded {
                challenge_id,
                max_attempts,
            } => write!(
                f,
                "link challenge '{challenge_id}' exceeded max attempts ({max_attempts})"
            ),
            Self::NonceMismatch { challenge_id } => {
                write!(
                    f,
                    "bilateral dual-nonce verification failed for challenge '{challenge_id}'"
                )
            }
            Self::OwnerEscalationDenied {
                target_actor,
                secondary_body_id,
                secondary_account_id,
            } => write!(
                f,
                "owner_escalation_guard: refused linking ({secondary_body_id}, {secondary_account_id}) into owner actor '{target_actor}' without explicit owner confirmation"
            ),
            Self::ConfigTakesPrecedence {
                body_id,
                account_id,
                configured_actor,
            } => write!(
                f,
                "({body_id}, {account_id}) is already statically bound to '{configured_actor}' in [[person]] config"
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

/// Append-only identity link log events (never contain raw secret nonces).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum IdentityLinkEvent {
    ActorLinkChallenge {
        challenge_id: String,
        primary_body_id: BodyId,
        primary_account_id: String,
        primary_actor: ActorId,
        secondary_body_id: BodyId,
        secondary_account_id: String,
        secondary_actor: ActorId,
        nonce1_blake3_hex: String,
        nonce2_blake3_hex: String,
        issued_at_ms: Time,
        expires_at_ms: Time,
        max_attempts: u32,
    },
    ActorLinked {
        challenge_id: String,
        body_id: BodyId,
        account_id: String,
        from_actor: ActorId,
        to_actor: ActorId,
        retroactive_merge: bool,
        owner_confirmed: bool,
        evidence_digest: String,
        linked_at_ms: Time,
    },
    ActorUnlinked {
        body_id: BodyId,
        account_id: String,
        actor: ActorId,
        reason: String,
        unlinked_at_ms: Time,
    },
}

/// Ephemeral out-of-band challenge nonces returned solely to the delivery transport
/// (never logged to the append-only event log).
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedLinkChallenge {
    pub challenge_id: String,
    pub primary_actor: ActorId,
    pub secondary_actor: ActorId,
    pub nonce_1: String,
    pub nonce_2: String,
    pub expires_at_ms: Time,
}

impl fmt::Debug for IssuedLinkChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedLinkChallenge")
            .field("challenge_id", &self.challenge_id)
            .field("primary_actor", &self.primary_actor)
            .field("secondary_actor", &self.secondary_actor)
            .field("nonce_1", &"[REDACTED]")
            .field("nonce_2", &"[REDACTED]")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Active prospective link record in the `E_link` projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveActorLink {
    pub body_id: BodyId,
    pub account_id: String,
    pub from_actor: ActorId,
    pub to_actor: ActorId,
    pub linked_at_ms: Time,
    pub retroactive_merge: bool,
    pub owner_confirmed: bool,
    pub evidence_digest: String,
}

impl ActiveActorLink {
    /// Converts this link into a protocol [`PrincipalLink`] record.
    #[must_use]
    pub fn to_protocol_link(&self) -> PrincipalLink {
        PrincipalLink {
            body_id: self.body_id.clone(),
            platform_actor: ActorId::new(&self.account_id),
            canonical_actor: self.to_actor.clone(),
            status: PrincipalLinkStatus::Linked,
            updated_at_ms: self.linked_at_ms,
            challenge_hash: Some(self.evidence_digest.clone()),
        }
    }
}

#[derive(Debug, Clone)]
struct ChallengeState {
    primary_body_id: BodyId,
    primary_account_id: String,
    primary_actor: ActorId,
    secondary_body_id: BodyId,
    secondary_account_id: String,
    secondary_actor: ActorId,
    nonce1_hash: blake3::Hash,
    nonce2_hash: blake3::Hash,
    expires_at_ms: Time,
    max_attempts: u32,
    attempts: u32,
    settled: bool,
}

#[derive(Debug)]
struct IdentityInner {
    config: IdentityConfig,
    /// Static `[[person]]` mapping: `(body_id, account_id) -> canonical ActorId`.
    config_map: BTreeMap<(BodyId, String), ActorId>,
    /// Explicitly configured owner `(body_id, account_id)` accounts (never elevated via runtime links).
    owner_accounts: BTreeSet<(BodyId, String)>,
    /// Explicitly configured owner `ActorId`s.
    owner_actors: BTreeSet<ActorId>,
    /// Active non-revoked runtime links (`E_link`).
    active_links: BTreeMap<(BodyId, String), ActiveActorLink>,
    /// Open/settled bilateral challenges indexed by `challenge_id`.
    challenges: BTreeMap<String, ChallengeState>,
    /// Append-only event log (in memory + optional JSONL file).
    events: Vec<IdentityLinkEvent>,
    log_path: Option<PathBuf>,
    next_challenge_seq: u64,
}

/// Thread-safe canonical `κ(b, a)` identity resolver and bilateral link manager (`4.3`).
#[derive(Clone, Debug)]
pub struct IdentityResolver {
    inner: Arc<RwLock<IdentityInner>>,
}

impl IdentityResolver {
    /// Constructs a new [`IdentityResolver`] from [`IdentityConfig`] and `[[person]]` mappings.
    pub fn new(
        identity_cfg: IdentityConfig,
        persons: &[PersonMappingConfig],
        log_path: Option<PathBuf>,
    ) -> Result<Self, IdentityError> {
        if identity_cfg.heuristic_merge_enabled {
            return Err(IdentityError::InvariantViolation(
                "heuristic_merge_enabled must always be false (T_mu = +inf)",
            ));
        }
        if !identity_cfg.owner_escalation_guard {
            return Err(IdentityError::InvariantViolation(
                "owner_escalation_guard must always be true",
            ));
        }

        let mut config_map = BTreeMap::new();
        let mut owner_accounts = BTreeSet::new();
        let mut owner_actors = BTreeSet::new();

        for person in persons {
            if person.is_owner {
                owner_actors.insert(person.actor_id.clone());
            }
            for acct in &person.accounts {
                let key = (acct.body_id.clone(), acct.account_id.clone());
                config_map.insert(key.clone(), person.actor_id.clone());
                if person.is_owner {
                    owner_accounts.insert(key);
                }
            }
        }

        let mut inner = IdentityInner {
            config: identity_cfg,
            config_map,
            owner_accounts,
            owner_actors,
            active_links: BTreeMap::new(),
            challenges: BTreeMap::new(),
            events: Vec::new(),
            log_path: log_path.clone(),
            next_challenge_seq: 1,
        };

        if let Some(path) = &log_path
            && path.exists()
        {
            let loaded = read_identity_events_jsonl(path)?;
            for ev in loaded {
                apply_identity_event(&mut inner, ev)?;
            }
        }

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
        })
    }

    /// Constructs an [`IdentityResolver`] from a parsed [`GatewayConfig`].
    pub fn from_gateway_config(
        config: &GatewayConfig,
        log_path: Option<PathBuf>,
    ) -> Result<Self, IdentityError> {
        Self::new(config.identity.clone(), &config.persons, log_path)
    }

    /// Reconstructs an [`IdentityResolver`] purely by replaying an append-only event stream.
    pub fn replay_from_events(
        identity_cfg: IdentityConfig,
        persons: &[PersonMappingConfig],
        events: &[IdentityLinkEvent],
    ) -> Result<Self, IdentityError> {
        let resolver = Self::new(identity_cfg, persons, None)?;
        {
            let mut st = resolver.inner.write().expect("identity lock");
            for ev in events {
                apply_identity_event(&mut st, ev.clone())?;
            }
        }
        Ok(resolver)
    }

    /// Returns `false` always (`T_μ = +∞`).
    #[must_use]
    pub const fn heuristic_merge_enabled(&self) -> bool {
        HEURISTIC_MERGE_ENABLED
    }

    /// Returns `+∞` (`T_μ = +∞`).
    #[must_use]
    pub const fn heuristic_merge_threshold_mu(&self) -> f64 {
        HEURISTIC_MERGE_THRESHOLD_MU
    }

    /// Resolves `(body_id, account_id)` in strict precedence order:
    /// `[[person]]` config -> non-revoked `actor_linked` -> `per_<hex32(BLAKE3("monoagens.actor.v1" || κ(b, a)))>`.
    #[must_use]
    pub fn resolve(&self, body_id: &BodyId, account_id: &str) -> ActorId {
        let st = self.inner.read().expect("identity lock");
        resolve_with_state(&st, body_id, account_id)
    }

    /// Resolves an ordered stream of `(body_id, account_id)` observations deterministically.
    #[must_use]
    pub fn resolve_stream<'a>(
        &self,
        stream: impl IntoIterator<Item = (&'a BodyId, &'a str)>,
    ) -> Vec<ActorId> {
        let st = self.inner.read().expect("identity lock");
        stream
            .into_iter()
            .map(|(b, a)| resolve_with_state(&st, b, a))
            .collect()
    }

    /// Returns `true` ONLY if `(body_id, account_id)` is directly configured as an owner
    /// account in `[[person]]`.
    ///
    /// Enforces `owner_escalation_guard = true`: a non-owner account linked at runtime can
    /// never acquire owner-only privileges.
    #[must_use]
    pub fn is_owner_account(&self, body_id: &BodyId, account_id: &str) -> bool {
        let st = self.inner.read().expect("identity lock");
        st.owner_accounts
            .contains(&(body_id.clone(), account_id.to_owned()))
    }

    /// Initiates a dual-nonce bilateral challenge to prospectively link `(secondary_body, secondary_account)`
    /// onto the canonical actor resolved for `(primary_body, primary_account)`.
    ///
    /// Appends an `actor_link_challenge` event containing ONLY cryptographic hashes of the
    /// `>= 128-bit` CSPRNG nonces (raw nonces never enter the log).
    pub fn initiate_bilateral_challenge(
        &self,
        primary_body_id: &BodyId,
        primary_account_id: &str,
        secondary_body_id: &BodyId,
        secondary_account_id: &str,
        now_ms: Time,
    ) -> Result<IssuedLinkChallenge, IdentityError> {
        let mut st = self.inner.write().expect("identity lock");

        if let Some(cfg_actor) = st
            .config_map
            .get(&(secondary_body_id.clone(), secondary_account_id.to_owned()))
        {
            return Err(IdentityError::ConfigTakesPrecedence {
                body_id: secondary_body_id.clone(),
                account_id: secondary_account_id.to_owned(),
                configured_actor: cfg_actor.clone(),
            });
        }

        let primary_actor = resolve_with_state(&st, primary_body_id, primary_account_id);
        let secondary_actor = resolve_with_state(&st, secondary_body_id, secondary_account_id);

        if st.config.owner_escalation_guard && st.owner_actors.contains(&primary_actor) {
            // Still allow creating challenge or refuse upfront?
            //Wait: let's check both initiation and completion; if primary is owner and secondary is not
            // an owner account, challenge creation may proceed if owner confirmation will be presented
            // at completion, OR completion rejects without owner_confirmed. Checking at completion
            // lets tests exercise the bilateral verification path + owner escalation refusal!
        }

        let nonce_1 = generate_nonce_128bit()?;
        let nonce_2 = generate_nonce_128bit()?;

        let seq = st.next_challenge_seq;
        st.next_challenge_seq = st.next_challenge_seq.saturating_add(1);
        let challenge_id = format!("lnk_{}_{}", now_ms.as_millis(), seq);

        let nonce1_hash = hash_link_nonce(&challenge_id, &nonce_1);
        let nonce2_hash = hash_link_nonce(&challenge_id, &nonce_2);
        let ttl_ms = st.config.link_challenge_ttl_s.saturating_mul(1_000);
        let expires_at_ms = now_ms.saturating_add_ms(ttl_ms);

        let event = IdentityLinkEvent::ActorLinkChallenge {
            challenge_id: challenge_id.clone(),
            primary_body_id: primary_body_id.clone(),
            primary_account_id: primary_account_id.to_owned(),
            primary_actor: primary_actor.clone(),
            secondary_body_id: secondary_body_id.clone(),
            secondary_account_id: secondary_account_id.to_owned(),
            secondary_actor: secondary_actor.clone(),
            nonce1_blake3_hex: nonce1_hash.to_hex().to_string(),
            nonce2_blake3_hex: nonce2_hash.to_hex().to_string(),
            issued_at_ms: now_ms,
            expires_at_ms,
            max_attempts: st.config.link_max_attempts,
        };

        append_and_persist_event(&mut st, event)?;

        Ok(IssuedLinkChallenge {
            challenge_id,
            primary_actor,
            secondary_actor,
            nonce_1,
            nonce_2,
            expires_at_ms,
        })
    }

    /// Completes a bilateral challenge by verifying BOTH `nonce_1` and `nonce_2` within
    /// `link_challenge_ttl_s` (`600 s` default) and `link_max_attempts` (`3` default),
    /// enforcing `owner_escalation_guard = true`.
    ///
    /// On success, appends `actor_linked` prospectively with `retroactive_merge = false`
    /// (by config default) and returns the linked canonical `ActorId`.
    pub fn verify_bilateral_challenge(
        &self,
        challenge_id: &str,
        echoed_nonce_1: &str,
        echoed_nonce_2: &str,
        owner_confirmed: bool,
        now_ms: Time,
    ) -> Result<ActorId, IdentityError> {
        let mut st = self.inner.write().expect("identity lock");

        let (
            primary_actor,
            secondary_body_id,
            secondary_account_id,
            secondary_actor,
            nonce1_hash,
            nonce2_hash,
            expires_at_ms,
            max_attempts,
            attempts,
            settled,
        ) = {
            let ch = st.challenges.get(challenge_id).ok_or_else(|| {
                IdentityError::ChallengeNotFound {
                    challenge_id: challenge_id.to_owned(),
                }
            })?;
            (
                ch.primary_actor.clone(),
                ch.secondary_body_id.clone(),
                ch.secondary_account_id.clone(),
                ch.secondary_actor.clone(),
                ch.nonce1_hash,
                ch.nonce2_hash,
                ch.expires_at_ms,
                ch.max_attempts,
                ch.attempts,
                ch.settled,
            )
        };

        if settled {
            return Err(IdentityError::ChallengeAlreadySettled {
                challenge_id: challenge_id.to_owned(),
            });
        }
        if now_ms > expires_at_ms {
            return Err(IdentityError::ChallengeExpired {
                challenge_id: challenge_id.to_owned(),
                expires_at_ms,
                now_ms,
            });
        }
        if attempts >= max_attempts {
            return Err(IdentityError::MaxAttemptsExceeded {
                challenge_id: challenge_id.to_owned(),
                max_attempts,
            });
        }

        if let Some(ch) = st.challenges.get_mut(challenge_id) {
            ch.attempts = ch.attempts.saturating_add(1);
        }

        let h1 = hash_link_nonce(challenge_id, echoed_nonce_1);
        let h2 = hash_link_nonce(challenge_id, echoed_nonce_2);
        // `blake3::Hash::eq` performs constant-time comparison.
        if h1 != nonce1_hash || h2 != nonce2_hash {
            return Err(IdentityError::NonceMismatch {
                challenge_id: challenge_id.to_owned(),
            });
        }

        // Enforce owner_escalation_guard = true (Absolute Line 1).
        if st.config.owner_escalation_guard
            && (st.owner_actors.contains(&primary_actor)
                || st.owner_actors.contains(&secondary_actor))
            && !owner_confirmed
        {
            return Err(IdentityError::OwnerEscalationDenied {
                target_actor: primary_actor,
                secondary_body_id,
                secondary_account_id,
            });
        }

        let mut ev_hasher = blake3::Hasher::new();
        ev_hasher.update(b"monoagens.link.evidence.v1");
        ev_hasher.update(challenge_id.as_bytes());
        ev_hasher.update(nonce1_hash.as_bytes());
        ev_hasher.update(nonce2_hash.as_bytes());
        let evidence_digest = format!("blake3:{}", ev_hasher.finalize().to_hex());

        let linked_event = IdentityLinkEvent::ActorLinked {
            challenge_id: challenge_id.to_owned(),
            body_id: secondary_body_id,
            account_id: secondary_account_id,
            from_actor: secondary_actor,
            to_actor: primary_actor.clone(),
            retroactive_merge: st.config.retroactive_merge,
            owner_confirmed,
            evidence_digest,
            linked_at_ms: now_ms,
        };

        append_and_persist_event(&mut st, linked_event)?;
        Ok(primary_actor)
    }

    /// Revokes an active runtime link by appending `actor_unlinked`.
    pub fn unlink_actor(
        &self,
        body_id: &BodyId,
        account_id: &str,
        reason: impl Into<String>,
        now_ms: Time,
    ) -> Result<(), IdentityError> {
        let mut st = self.inner.write().expect("identity lock");
        let current = resolve_with_state(&st, body_id, account_id);
        let event = IdentityLinkEvent::ActorUnlinked {
            body_id: body_id.clone(),
            account_id: account_id.to_owned(),
            actor: current,
            reason: reason.into(),
            unlinked_at_ms: now_ms,
        };
        append_and_persist_event(&mut st, event)
    }

    /// Returns whether `(body_id, account_id)` may view historical private items recorded for
    /// `historical_actor` at `event_ts_ms`.
    ///
    /// When `retroactive_merge = false` (default), linking `(body_id, account_id) -> P_1` NEVER
    /// grants access to pre-link (`event_ts_ms < linked_at_ms`) private items of `P_1`, nor
    /// exposes pre-link private items of `P_2` (`from_actor`) to `P_1`.
    #[must_use]
    pub fn can_access_historical_actor_item(
        &self,
        viewer_body_id: &BodyId,
        viewer_account_id: &str,
        historical_actor: &ActorId,
        event_ts_ms: Time,
    ) -> bool {
        let st = self.inner.read().expect("identity lock");
        let key = (viewer_body_id.clone(), viewer_account_id.to_owned());

        // Static config mapping is present from t = 0.
        if let Some(cfg_actor) = st.config_map.get(&key) {
            return cfg_actor == historical_actor;
        }

        if let Some(link) = st.active_links.get(&key) {
            if event_ts_ms >= link.linked_at_ms {
                return &link.to_actor == historical_actor;
            }
            // Pre-link item: if retroactive_merge is false, viewer only matches its pre-link identity.
            if !link.retroactive_merge {
                return &link.from_actor == historical_actor;
            }
            return &link.to_actor == historical_actor || &link.from_actor == historical_actor;
        }

        let derived = derive_per_actor_id(viewer_body_id, viewer_account_id);
        &derived == historical_actor
    }

    /// Returns a snapshot of all append-only [`IdentityLinkEvent`]s recorded so far.
    #[must_use]
    pub fn events(&self) -> Vec<IdentityLinkEvent> {
        self.inner.read().expect("identity lock").events.clone()
    }

    /// Returns a snapshot of all currently active prospective links.
    #[must_use]
    pub fn active_links(&self) -> Vec<ActiveActorLink> {
        self.inner
            .read()
            .expect("identity lock")
            .active_links
            .values()
            .cloned()
            .collect()
    }
}

impl IdentitySeam for IdentityResolver {
    fn resolve_actor(&self, body_id: &BodyId, account_id: &ActorId) -> ActorId {
        self.resolve(body_id, account_id.as_str())
    }
}

fn resolve_with_state(st: &IdentityInner, body_id: &BodyId, account_id: &str) -> ActorId {
    let key = (body_id.clone(), account_id.to_owned());
    // 1. [[person]] config wins over everything.
    if let Some(cfg_actor) = st.config_map.get(&key) {
        return cfg_actor.clone();
    }
    // 2. Non-revoked runtime link.
    if let Some(link) = st.active_links.get(&key) {
        return link.to_actor.clone();
    }
    // 3. Canonical κ(b, a) BLAKE3 derivation.
    derive_per_actor_id(body_id, account_id)
}

fn generate_nonce_128bit() -> Result<String, IdentityError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| IdentityError::EntropyFailure(e.to_string()))?;
    let mut out = String::with_capacity(32);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    Ok(out)
}

fn hash_link_nonce(challenge_id: &str, nonce: &str) -> blake3::Hash {
    let cid = challenge_id.as_bytes();
    let n = nonce.as_bytes();
    let mut hasher = blake3::Hasher::new();
    hasher.update(LINK_NONCE_DOMAIN_PREFIX);
    hasher.update(&u32::try_from(cid.len()).unwrap_or(u32::MAX).to_le_bytes());
    hasher.update(cid);
    hasher.update(&u32::try_from(n.len()).unwrap_or(u32::MAX).to_le_bytes());
    hasher.update(n);
    hasher.finalize()
}

fn append_and_persist_event(
    st: &mut IdentityInner,
    event: IdentityLinkEvent,
) -> Result<(), IdentityError> {
    if let Some(path) = &st.log_path {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| IdentityError::Io {
                path: parent.display().to_string(),
                detail: e.to_string(),
            })?;
        }
        let line = serde_json::to_string(&event).map_err(|e| IdentityError::Parse {
            detail: e.to_string(),
        })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| IdentityError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            })?;
        writeln!(file, "{line}").map_err(|e| IdentityError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        file.sync_data().map_err(|e| IdentityError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    apply_identity_event(st, event)
}

fn apply_identity_event(
    st: &mut IdentityInner,
    event: IdentityLinkEvent,
) -> Result<(), IdentityError> {
    match &event {
        IdentityLinkEvent::ActorLinkChallenge {
            challenge_id,
            primary_body_id,
            primary_account_id,
            primary_actor,
            secondary_body_id,
            secondary_account_id,
            secondary_actor,
            nonce1_blake3_hex,
            nonce2_blake3_hex,
            expires_at_ms,
            max_attempts,
            ..
        } => {
            let n1 =
                blake3::Hash::from_hex(nonce1_blake3_hex).map_err(|e| IdentityError::Parse {
                    detail: format!("invalid nonce1 hash hex: {e}"),
                })?;
            let n2 =
                blake3::Hash::from_hex(nonce2_blake3_hex).map_err(|e| IdentityError::Parse {
                    detail: format!("invalid nonce2 hash hex: {e}"),
                })?;
            st.challenges.insert(
                challenge_id.clone(),
                ChallengeState {
                    primary_body_id: primary_body_id.clone(),
                    primary_account_id: primary_account_id.clone(),
                    primary_actor: primary_actor.clone(),
                    secondary_body_id: secondary_body_id.clone(),
                    secondary_account_id: secondary_account_id.clone(),
                    secondary_actor: secondary_actor.clone(),
                    nonce1_hash: n1,
                    nonce2_hash: n2,
                    expires_at_ms: *expires_at_ms,
                    max_attempts: *max_attempts,
                    attempts: 0,
                    settled: false,
                },
            );
            st.next_challenge_seq = st.next_challenge_seq.saturating_add(1);
        }
        IdentityLinkEvent::ActorLinked {
            challenge_id,
            body_id,
            account_id,
            from_actor,
            to_actor,
            retroactive_merge,
            owner_confirmed,
            evidence_digest,
            linked_at_ms,
        } => {
            if let Some(ch) = st.challenges.get_mut(challenge_id) {
                ch.settled = true;
            }
            // Also keep primary_body/primary_account in challenge state accessible if needed.
            let _ = st
                .challenges
                .get(challenge_id)
                .map(|c| (&c.primary_body_id, &c.primary_account_id));
            let key = (body_id.clone(), account_id.clone());
            // Never allow a runtime link to overwrite an owner escalation without confirmation.
            let involves_owner =
                st.owner_actors.contains(to_actor) || st.owner_actors.contains(from_actor);
            if st.config.owner_escalation_guard && involves_owner && !*owner_confirmed {
                return Err(IdentityError::OwnerEscalationDenied {
                    target_actor: to_actor.clone(),
                    secondary_body_id: body_id.clone(),
                    secondary_account_id: account_id.clone(),
                });
            }
            st.active_links.insert(
                key,
                ActiveActorLink {
                    body_id: body_id.clone(),
                    account_id: account_id.clone(),
                    from_actor: from_actor.clone(),
                    to_actor: to_actor.clone(),
                    linked_at_ms: *linked_at_ms,
                    retroactive_merge: *retroactive_merge,
                    owner_confirmed: *owner_confirmed,
                    evidence_digest: evidence_digest.clone(),
                },
            );
        }
        IdentityLinkEvent::ActorUnlinked {
            body_id,
            account_id,
            ..
        } => {
            st.active_links
                .remove(&(body_id.clone(), account_id.clone()));
        }
    }
    st.events.push(event);
    Ok(())
}

fn read_identity_events_jsonl(path: &Path) -> Result<Vec<IdentityLinkEvent>, IdentityError> {
    let file = fs::File::open(path).map_err(|e| IdentityError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line_res) in reader.lines().enumerate() {
        let line = line_res.map_err(|e| IdentityError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let ev: IdentityLinkEvent =
            serde_json::from_str(trimmed).map_err(|e| IdentityError::Parse {
                detail: format!("line {}: {e}", idx + 1),
            })?;
        out.push(ev);
    }
    Ok(out)
}
