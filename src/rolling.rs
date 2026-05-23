use std::collections::BTreeMap;

use crate::report::compute_buckets;
use crate::state::Finding;

/// A minimal per-message summary stored in the rolling window.
#[derive(Debug, Clone)]
struct MessageSlot {
    msg_type: u8,
    #[allow(dead_code)]
    findings: Vec<Finding>,
    malformed: bool,
}

/// Rolling window summary of the last N parsed BMP messages.
///
/// Accepts messages one at a time and maintains bounded aggregate
/// counts. When an old message rolls out of the window, its
/// contributions to totals are decremented.
#[derive(Debug, Clone)]
pub struct RollingSummary {
    window_messages: usize,
    total_seen: u64,
    window_seen: u64,
    slots: Vec<MessageSlot>,
    next_slot: usize,
    by_type: BTreeMap<u8, u64>,
    malformed_messages: u64,
}

impl RollingSummary {
    pub fn new(window_messages: usize) -> Self {
        RollingSummary {
            window_messages,
            total_seen: 0,
            window_seen: 0,
            slots: Vec::new(),
            next_slot: 0,
            by_type: BTreeMap::new(),
            malformed_messages: 0,
        }
    }

    pub fn total_seen(&self) -> u64 {
        self.total_seen
    }

    pub fn window_seen(&self) -> u64 {
        self.window_seen
    }

    pub fn by_type(&self) -> &BTreeMap<u8, u64> {
        &self.by_type
    }

    pub fn malformed_messages(&self) -> u64 {
        self.malformed_messages
    }

    /// Compute findings buckets from all findings currently in the window.
    #[allow(dead_code)]
    pub(crate) fn findings_buckets(&self) -> crate::report::FindingsBuckets {
        if self.window_messages == 0 || self.window_seen == 0 {
            return compute_buckets(&[]);
        }
        let count = self.window_seen.min(self.window_messages as u64) as usize;
        let mut all = Vec::new();
        let start = if self.window_seen as usize <= self.window_messages {
            0
        } else {
            self.next_slot
        };
        for i in 0..count {
            let idx = (start + i) % self.window_messages;
            all.extend_from_slice(&self.slots[idx].findings);
        }
        compute_buckets(&all)
    }

    /// Add a parsed message to the rolling window.
    ///
    /// `msg_type` is the raw BMP message type byte (0–6, or unknown).
    /// `malformed` indicates a frame-level error.
    /// `findings` are per-message findings from parsing and state tracking.
    pub fn push(&mut self, msg_type: u8, malformed: bool, findings: Vec<Finding>) {
        self.total_seen += 1;

        if self.window_messages == 0 {
            return;
        }

        // Initialize ring buffer on first push
        if self.slots.is_empty() {
            self.slots = vec![
                MessageSlot {
                    msg_type: 0,
                    findings: Vec::new(),
                    malformed: false,
                };
                self.window_messages
            ];
        }

        // Evict old slot if window is full
        if self.window_seen as usize >= self.window_messages {
            let old = &self.slots[self.next_slot];
            *self.by_type.entry(old.msg_type).or_insert(0) -= 1;
            if old.malformed {
                self.malformed_messages -= 1;
            }
        } else {
            self.window_seen += 1;
        }

        // Insert new slot
        self.slots[self.next_slot] = MessageSlot {
            msg_type,
            findings,
            malformed,
        };
        *self.by_type.entry(msg_type).or_insert(0) += 1;
        if malformed {
            self.malformed_messages += 1;
        }
        self.next_slot = (self.next_slot + 1) % self.window_messages;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_summary() {
        let rs = RollingSummary::new(10);
        assert_eq!(rs.total_seen(), 0);
        assert_eq!(rs.window_seen(), 0);
        assert_eq!(rs.malformed_messages(), 0);
        assert!(rs.by_type().is_empty());
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.parse_errors, 0);
    }

    #[test]
    fn test_below_window() {
        let mut rs = RollingSummary::new(10);
        rs.push(0, false, vec![]); // Route Monitoring
        rs.push(3, false, vec![]); // Peer Up
        rs.push(0, false, vec![]);
        assert_eq!(rs.total_seen(), 3);
        assert_eq!(rs.window_seen(), 3);
        assert_eq!(*rs.by_type().get(&0).unwrap_or(&0), 2);
        assert_eq!(*rs.by_type().get(&3).unwrap_or(&0), 1);
    }

    #[test]
    fn test_rolls_off_old_entries() {
        let mut rs = RollingSummary::new(3);
        rs.push(0, false, vec![]);
        rs.push(0, false, vec![]);
        rs.push(3, false, vec![]);
        assert_eq!(rs.window_seen(), 3);
        assert_eq!(*rs.by_type().get(&3).unwrap_or(&0), 1);
        // Push 4th — oldest rolls off
        rs.push(4, false, vec![]); // Initiation (rolls off one type-0)
        assert_eq!(rs.window_seen(), 3);
        assert_eq!(rs.total_seen(), 4);
        assert_eq!(*rs.by_type().get(&0).unwrap_or(&0), 1); // was 2, now 1
        assert_eq!(*rs.by_type().get(&3).unwrap_or(&0), 1);
        assert_eq!(*rs.by_type().get(&4).unwrap_or(&0), 1);
    }

    #[test]
    fn test_malformed_rolls_off() {
        let mut rs = RollingSummary::new(2);
        rs.push(0, true, vec![]);
        rs.push(0, false, vec![]);
        assert_eq!(rs.malformed_messages(), 1);
        rs.push(0, false, vec![]); // rolls off the malformed one
        assert_eq!(rs.malformed_messages(), 0);
    }

    #[test]
    fn test_window_size_zero() {
        let mut rs = RollingSummary::new(0);
        rs.push(0, false, vec![]);
        assert_eq!(rs.total_seen(), 1);
        assert_eq!(rs.window_seen(), 0);
        assert!(rs.by_type().is_empty());
    }

    #[test]
    fn test_findings_roll_off() {
        use crate::state::{Finding, Severity};
        let f1 = Finding {
            severity: Severity::Warn,
            rule: "test_rule".into(),
            offset: None,
            peer: None,
            message: "m1".into(),
        };
        let f2 = Finding {
            severity: Severity::Warn,
            rule: "test_rule".into(),
            offset: None,
            peer: None,
            message: "m2".into(),
        };

        let mut rs = RollingSummary::new(2);
        rs.push(0, false, vec![f1.clone()]);
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.other_findings, 1);

        rs.push(0, false, vec![f2.clone()]);
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.other_findings, 2);

        // Push 3rd — oldest findings roll off
        rs.push(0, false, vec![]);
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.other_findings, 1); // only f2 remains
    }
}
