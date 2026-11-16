//! Cognitive loop turn execution, 3-class queue (`A > B > C`), and crash recovery.

pub mod queue;

pub use queue::{
    CLASS_A_CAPACITY, CLASS_B_CAPACITY, CLASS_C_CAPACITY, QueueClass, QueueOverflow,
    ThreeClassQueue,
};
