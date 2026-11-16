//! Synchronous sans-IO ports (`Store`, `Doors`) connecting Core to the outer Gateway.

use core::fmt;

use monoagens_protocol::{Action, EmbeddingTag, EventId, Percept, PlaceId, VisibleScope};
use serde::{Deserialize, Serialize};

use crate::context::{
    ContextSections, DroppedContextItem, ValveSelfSnapshot, compute_static_prefix_digest,
};
use crate::event::{EventFormatError, EventRecord, LogSeq};

/// Errors returned by a [`Store`] implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    AppendFailed(String),
    NotFound(EventId),
    EventFormat(EventFormatError),
    SnapshotCorrupted { snapshot_id: String, detail: String },
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppendFailed(reason) => write!(f, "store append failed: {reason}"),
            Self::NotFound(id) => write!(f, "event {id} not found"),
            Self::EventFormat(err) => write!(f, "{err}"),
            Self::SnapshotCorrupted {
                snapshot_id,
                detail,
            } => write!(f, "snapshot '{snapshot_id}' corrupted: {detail}"),
            Self::Backend(reason) => write!(f, "store backend error: {reason}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<EventFormatError> for StoreError {
    fn from(err: EventFormatError) -> Self {
        Self::EventFormat(err)
    }
}

/// Checksummed state snapshot persisted through [`Store::snapshot`] and loaded by [`Store::latest_checkpoint`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCheckpoint {
    /// Opaque snapshot identifier included in checksum mismatch diagnostics.
    pub snapshot_id: String,
    /// Monotonic log position covered by this snapshot; events with `seq > cursor` form the replay tail.
    pub cursor: LogSeq,
    /// Two-Epoch Replay epoch counter (`0` prior to any `hard_forget`).
    pub epoch: u64,
    /// Serialized snapshot payload bytes.
    pub bytes: Vec<u8>,
    /// Standard BLAKE3-256 checksum over `bytes`.
    pub blake3: [u8; 32],
}

/// Purpose of a model invocation requested by Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPurpose {
    Reason,
    Classify,
    Consolidate,
}

/// Bounded context plan passed through `Doors::context` and emitted on `Effect::Model`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContextPlan {
    pub trigger_event_id: Option<EventId>,
    pub place: Option<PlaceId>,
    pub visible_scope: Option<VisibleScope>,
    pub items: Vec<EventId>,
    #[serde(default)]
    pub sections: ContextSections,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_items: Vec<DroppedContextItem>,
    #[serde(default)]
    pub static_prefix_digest: [u8; 32],
    /// Optional opaque `ValveSelfSnapshot` attached by `Doors::context` (`7.6`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valve_self_snapshot: Option<ValveSelfSnapshot>,
}

impl ContextPlan {
    /// Recomputes `static_prefix_digest` over Slot 0, static core capabilities, optional
    /// `CoreSelfSnapshot`, and optional `ValveSelfSnapshot`.
    pub fn refresh_static_prefix_digest(&mut self) {
        self.static_prefix_digest =
            compute_static_prefix_digest(&self.sections, self.valve_self_snapshot.as_ref());
    }
}

/// Synchronous persistence and candidate-retrieval port implemented outside Core.
pub trait Store {
    /// Appends an event record durably before Core processes or acts on it.
    ///
    /// Implementations MUST assign a strictly monotonic [`LogSeq`] on append.
    fn append(&mut self, record: EventRecord) -> Result<EventId, StoreError>;

    /// Reads recorded events in monotonic [`LogSeq`] order after `after` (or from the start if `None`).
    fn read_in_order(
        &self,
        after: Option<EventId>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError>;

    /// Reads recorded events in monotonic [`LogSeq`] order strictly after `after` (or from start if `None`).
    fn read_after_seq(
        &self,
        after: Option<LogSeq>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError>;

    /// Fetches a single event record by id (upcasting older supported formats in memory).
    fn get(&self, id: EventId) -> Result<Option<EventRecord>, StoreError>;

    /// Persists a `blake3`-checksummed state snapshot at `checkpoint.cursor`.
    fn snapshot(&mut self, checkpoint: SnapshotCheckpoint) -> Result<(), StoreError>;

    /// Loads the latest persisted snapshot checkpoint, if any exists.
    fn latest_checkpoint(&self) -> Result<Option<SnapshotCheckpoint>, StoreError>;

    /// Returns candidate event ids ranked by vector similarity within the matching embedding tag.
    fn candidates_by_vector(
        &self,
        tag: &EmbeddingTag,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(EventId, f32)>, StoreError>;

    /// Returns candidate event ids matching the text of the event identified by `query_event_id`.
    fn candidates_by_text(
        &self,
        query_event_id: EventId,
        limit: usize,
    ) -> Result<Vec<EventId>, StoreError>;

    /// Returns candidate event ids matching the text of `query_event_id`, partitioned by `tag`
    /// where the store reports embedding partition tags (`5.4`).
    fn candidates_by_text_partitioned(
        &self,
        query_event_id: EventId,
        tag: Option<&EmbeddingTag>,
        limit: usize,
    ) -> Result<Vec<EventId>, StoreError> {
        let _ = tag;
        self.candidates_by_text(query_event_id, limit)
    }

    /// Releases stored vectors associated with `event_id` (e.g. Petrov fade or `hard_forget`).
    fn release_vector(&mut self, event_id: EventId) -> Result<(), StoreError>;
}

/// Decision returned by one of the three Gateway doors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoorDecision {
    Allow,
    Strip {
        removed_ids: Vec<EventId>,
        rule: String,
    },
    Hold {
        rule: String,
        reason: String,
    },
    Refuse {
        rule: String,
        reason: String,
    },
}

/// Three synchronous policy/plugin hook points invoked during a Core turn.
pub trait Doors {
    /// Step 3: inbound percept inspection.
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision;

    /// Step 6: assembled context inspection before model invocation.
    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision;

    /// Step 7: outbound action inspection before dispatch.
    fn action(&mut self, action: &mut Action) -> DoorDecision;
}

/// Default permissive no-op [`Doors`] implementation (Core runs autonomously with no valve).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopDoors;

impl Doors for NoopDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        DoorDecision::Allow
    }

    fn context(&mut self, _plan: &mut ContextPlan) -> DoorDecision {
        DoorDecision::Allow
    }

    fn action(&mut self, _action: &mut Action) -> DoorDecision {
        DoorDecision::Allow
    }
}
