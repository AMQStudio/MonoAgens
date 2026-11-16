//! Three-class bounded ingress priority queue (`A=256 > B=512 > C=256`).

use core::fmt;
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Capacity of Class A (Urgent Control: cancel, barge-in, preemption).
pub const CLASS_A_CAPACITY: usize = 256;

/// Capacity of Class B (Conversational: percepts, model completions, normal receipts).
pub const CLASS_B_CAPACITY: usize = 512;

/// Capacity of Class C (Background / Sleep: idle ticks, consolidation).
pub const CLASS_C_CAPACITY: usize = 256;

/// Bounded ingress queue priority class drained strictly in `A > B > C` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum QueueClass {
    /// Urgent control (capacity 256).
    A,
    /// Conversational (capacity 512).
    B,
    /// Background / Sleep (capacity 256).
    C,
}

impl QueueClass {
    /// Returns the fixed bounded capacity for this queue class.
    #[must_use]
    pub const fn capacity(self) -> usize {
        match self {
            Self::A => CLASS_A_CAPACITY,
            Self::B => CLASS_B_CAPACITY,
            Self::C => CLASS_C_CAPACITY,
        }
    }

    /// Returns a human-readable class label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "Class A (Urgent Control)",
            Self::B => "Class B (Conversational)",
            Self::C => "Class C (Background/Sleep)",
        }
    }
}

impl fmt::Display for QueueClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Typed back-pressure error emitted when a bounded queue class is at capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueOverflow {
    pub class: QueueClass,
    pub capacity: usize,
    pub current_len: usize,
}

impl fmt::Display for QueueOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} queue overflow: {}/{} items",
            self.class, self.current_len, self.capacity
        )
    }
}

impl std::error::Error for QueueOverflow {}

/// Single-consumer bounded priority queue with strict `A > B > C` FIFO drain semantics.
#[derive(Debug, Clone, Default)]
pub struct ThreeClassQueue<T> {
    class_a: VecDeque<T>,
    class_b: VecDeque<T>,
    class_c: VecDeque<T>,
}

impl<T> ThreeClassQueue<T> {
    /// Creates an empty 3-class queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            class_a: VecDeque::new(),
            class_b: VecDeque::new(),
            class_c: VecDeque::new(),
        }
    }

    /// Enqueues `item` into `class`, returning [`QueueOverflow`] instead of silently dropping.
    pub fn push(&mut self, class: QueueClass, item: T) -> Result<(), QueueOverflow> {
        let cap = class.capacity();
        let buf = self.buf_mut(class);
        if buf.len() >= cap {
            return Err(QueueOverflow {
                class,
                capacity: cap,
                current_len: buf.len(),
            });
        }
        buf.push_back(item);
        Ok(())
    }

    /// Dequeues the next item strictly in `A > B > C` order, FIFO within each class.
    pub fn pop(&mut self) -> Option<(QueueClass, T)> {
        if let Some(item) = self.class_a.pop_front() {
            return Some((QueueClass::A, item));
        }
        if let Some(item) = self.class_b.pop_front() {
            return Some((QueueClass::B, item));
        }
        self.class_c.pop_front().map(|item| (QueueClass::C, item))
    }

    /// Peeks at the next item that `pop` would return without removing it.
    #[must_use]
    pub fn peek(&self) -> Option<(QueueClass, &T)> {
        if let Some(item) = self.class_a.front() {
            return Some((QueueClass::A, item));
        }
        if let Some(item) = self.class_b.front() {
            return Some((QueueClass::B, item));
        }
        self.class_c.front().map(|item| (QueueClass::C, item))
    }

    /// Total number of queued items across all three classes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.class_a.len() + self.class_b.len() + self.class_c.len()
    }

    /// Returns `true` if all three classes are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.class_a.is_empty() && self.class_b.is_empty() && self.class_c.is_empty()
    }

    /// Number of queued items in `class`.
    #[must_use]
    pub fn class_len(&self, class: QueueClass) -> usize {
        self.buf(class).len()
    }

    /// Remaining capacity in `class`.
    #[must_use]
    pub fn remaining_capacity(&self, class: QueueClass) -> usize {
        class.capacity().saturating_sub(self.class_len(class))
    }

    fn buf(&self, class: QueueClass) -> &VecDeque<T> {
        match class {
            QueueClass::A => &self.class_a,
            QueueClass::B => &self.class_b,
            QueueClass::C => &self.class_c,
        }
    }

    fn buf_mut(&mut self, class: QueueClass) -> &mut VecDeque<T> {
        match class {
            QueueClass::A => &mut self.class_a,
            QueueClass::B => &mut self.class_b,
            QueueClass::C => &mut self.class_c,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CLASS_A_CAPACITY, CLASS_B_CAPACITY, CLASS_C_CAPACITY, QueueClass, QueueOverflow,
        ThreeClassQueue,
    };

    #[test]
    fn capacities_match_spec() {
        assert_eq!(CLASS_A_CAPACITY, 256);
        assert_eq!(CLASS_B_CAPACITY, 512);
        assert_eq!(CLASS_C_CAPACITY, 256);
        assert_eq!(QueueClass::A.capacity(), 256);
        assert_eq!(QueueClass::B.capacity(), 512);
        assert_eq!(QueueClass::C.capacity(), 256);
    }

    #[test]
    fn drains_strictly_a_before_b_before_c_fifo_per_class() {
        let mut q = ThreeClassQueue::new();
        q.push(QueueClass::C, "c1").unwrap();
        q.push(QueueClass::B, "b1").unwrap();
        q.push(QueueClass::C, "c2").unwrap();
        q.push(QueueClass::A, "a1").unwrap();
        q.push(QueueClass::B, "b2").unwrap();
        q.push(QueueClass::A, "a2").unwrap();

        let drained: Vec<_> = std::iter::from_fn(|| q.pop()).collect();
        assert_eq!(
            drained,
            vec![
                (QueueClass::A, "a1"),
                (QueueClass::A, "a2"),
                (QueueClass::B, "b1"),
                (QueueClass::B, "b2"),
                (QueueClass::C, "c1"),
                (QueueClass::C, "c2"),
            ]
        );
    }

    #[test]
    fn overflow_returns_typed_error_without_dropping_existing_items() {
        let mut q = ThreeClassQueue::new();
        for i in 0..CLASS_A_CAPACITY {
            q.push(QueueClass::A, i).expect("within capacity");
        }
        let err = q
            .push(QueueClass::A, 999)
            .expect_err("must reject overflow");
        assert_eq!(
            err,
            QueueOverflow {
                class: QueueClass::A,
                capacity: CLASS_A_CAPACITY,
                current_len: CLASS_A_CAPACITY,
            }
        );
        assert_eq!(q.class_len(QueueClass::A), CLASS_A_CAPACITY);
        assert_eq!(q.pop(), Some((QueueClass::A, 0)));
    }
}
