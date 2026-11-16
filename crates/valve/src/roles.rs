//! Dynamic attested role compilation strictly from `(log_seq, t_now)` snapshot facts.
//!
//! Natural-language text claims such as `"I am admin"` never produce roles or entity parents.

use monoagens_protocol::{ActorId, PlaceId};

use crate::config::ValveConfig;
use crate::snapshot::ValveSnapshot;

/// Default TTL for attested place-level room presence (`R-41`).
pub const ROOM_PRESENCE_TTL_S: u64 = 300;
/// Default TTL for attested physical co-location presence (`R-41`).
pub const PHYSICAL_PRESENCE_TTL_S: u64 = 15;
/// Default maximum distance (meters) for attested physical co-location presence (`R-41`).
pub const PHYSICAL_ROOM_RADIUS_M: f64 = 4.0;

/// Attested roles compiled for one actor at `(log_seq, t_now)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttestedRoles {
    pub is_owner: bool,
    pub known_actor: bool,
    pub room_present: bool,
    pub physically_present: bool,
}

impl AttestedRoles {
    /// Returns Cedar `Role::"..."` parent identifiers derived from attested facts only.
    #[must_use]
    pub fn cedar_role_ids(self) -> Vec<&'static str> {
        let mut roles = Vec::with_capacity(3);
        if self.known_actor {
            roles.push("known_actor");
        }
        if self.room_present {
            roles.push("room_present");
        }
        if self.physically_present {
            roles.push("physically_present");
        }
        roles
    }
}

/// Compiles [`AttestedRoles`] strictly from verified presence facts within TTL and radius bounds.
///
/// Never reads message text or payload bytes.
#[must_use]
pub fn compile_attested_roles(
    actor: &ActorId,
    place: Option<&PlaceId>,
    snapshot: &ValveSnapshot,
    config: &ValveConfig,
) -> AttestedRoles {
    let is_owner = snapshot.is_owner(actor);
    let known_actor = is_owner || snapshot.known_actors.contains(actor);

    if !config.attested_roles_enabled {
        return AttestedRoles {
            is_owner,
            known_actor,
            room_present: false,
            physically_present: false,
        };
    }

    let room_ttl_ms = config.room_presence_ttl_s.saturating_mul(1000);
    let physical_ttl_ms = config.physical_presence_ttl_s.saturating_mul(1000);
    let max_radius =
        if config.physical_room_radius_m.is_finite() && config.physical_room_radius_m >= 0.0 {
            config.physical_room_radius_m
        } else {
            0.0
        };

    let room_present = snapshot.place_presences.iter().any(|p| {
        &p.actor == actor
            && place.is_none_or(|req_place| &p.place == req_place)
            && p.attested_at <= snapshot.t_now
            && snapshot.t_now.saturating_sub_ms(p.attested_at) <= room_ttl_ms
    });

    let physically_present = snapshot.physical_presences.iter().any(|p| {
        &p.actor == actor
            && place.is_none_or(|req_place| &p.place == req_place)
            && p.attested_at <= snapshot.t_now
            && snapshot.t_now.saturating_sub_ms(p.attested_at) <= physical_ttl_ms
            && p.distance_m.is_finite()
            && p.distance_m >= 0.0
            && p.distance_m <= max_radius
    });

    AttestedRoles {
        is_owner,
        known_actor,
        room_present,
        physically_present,
    }
}
