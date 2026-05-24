use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::raw_bmp::BmpMessageType;
use crate::report::compute_buckets;
use crate::state::{Finding, PeerKey};

/// A minimal per-message summary stored in the rolling window.
#[derive(Debug, Clone)]
struct MessageSlot {
    msg_type: u8,
    #[allow(dead_code)]
    findings: Vec<Finding>,
    malformed: bool,
    peer_key: Option<PeerKey>,
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
    peer_counts: HashMap<PeerKey, usize>,
    metadata: Option<crate::obmp_reader::OpenBmpMetadata>,
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
            peer_counts: HashMap::new(),
            metadata: None,
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

    pub fn peers_observed(&self) -> usize {
        self.peer_counts.len()
    }

    pub fn set_metadata(&mut self, meta: crate::obmp_reader::OpenBmpMetadata) {
        if self.metadata.is_none() {
            self.metadata = Some(meta);
        }
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
    /// `peer_key` identifies the BGP peer associated with this message, if any.
    pub fn push(
        &mut self,
        msg_type: u8,
        malformed: bool,
        findings: Vec<Finding>,
        peer_key: Option<PeerKey>,
    ) {
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
                    peer_key: None,
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
            if let Some(ref pk) = old.peer_key {
                if let std::collections::hash_map::Entry::Occupied(mut e) =
                    self.peer_counts.entry(pk.clone())
                {
                    *e.get_mut() -= 1;
                    if *e.get() == 0 {
                        e.remove();
                    }
                }
            }
        } else {
            self.window_seen += 1;
        }

        // Insert new slot
        let pk_for_count = peer_key.clone();
        self.slots[self.next_slot] = MessageSlot {
            msg_type,
            findings,
            malformed,
            peer_key,
        };
        *self.by_type.entry(msg_type).or_insert(0) += 1;
        if malformed {
            self.malformed_messages += 1;
        }
        if let Some(pk) = pk_for_count {
            *self.peer_counts.entry(pk).or_insert(0) += 1;
        }
        self.next_slot = (self.next_slot + 1) % self.window_messages;
    }

    /// Print a single JSON line to stdout with current window state.
    pub fn emit_json(&self, elapsed_ms: u64) {
        let buckets = self.findings_buckets();

        let by_type: BTreeMap<String, u64> = self
            .by_type
            .iter()
            .map(|(type_id, count)| {
                let name = BmpMessageType::from_u8(*type_id)
                    .map(|t| t.as_str().to_string())
                    .unwrap_or_else(|| format!("Unknown({type_id})"));
                (name, *count)
            })
            .collect();

        #[derive(Serialize)]
        struct JsonOutput<'a> {
            window_messages: usize,
            total_seen: u64,
            elapsed_ms: u64,
            by_type: BTreeMap<String, u64>,
            peers_observed: usize,
            findings_buckets: &'a crate::report::FindingsBuckets,
            malformed_messages: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            openbmp_metadata: Option<&'a crate::obmp_reader::OpenBmpMetadata>,
        }

        let output = JsonOutput {
            window_messages: self.window_messages,
            total_seen: self.total_seen,
            elapsed_ms,
            by_type,
            peers_observed: self.peer_counts.len(),
            findings_buckets: &buckets,
            malformed_messages: self.malformed_messages,
            openbmp_metadata: self.metadata.as_ref(),
        };

        println!("{}", serde_json::to_string(&output).unwrap());
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
        rs.push(0, false, vec![], None); // Route Monitoring
        rs.push(3, false, vec![], None); // Peer Up
        rs.push(0, false, vec![], None);
        assert_eq!(rs.total_seen(), 3);
        assert_eq!(rs.window_seen(), 3);
        assert_eq!(*rs.by_type().get(&0).unwrap_or(&0), 2);
        assert_eq!(*rs.by_type().get(&3).unwrap_or(&0), 1);
    }

    #[test]
    fn test_rolls_off_old_entries() {
        let mut rs = RollingSummary::new(3);
        rs.push(0, false, vec![], None);
        rs.push(0, false, vec![], None);
        rs.push(3, false, vec![], None);
        assert_eq!(rs.window_seen(), 3);
        assert_eq!(*rs.by_type().get(&3).unwrap_or(&0), 1);
        // Push 4th — oldest rolls off
        rs.push(4, false, vec![], None); // Initiation (rolls off one type-0)
        assert_eq!(rs.window_seen(), 3);
        assert_eq!(rs.total_seen(), 4);
        assert_eq!(*rs.by_type().get(&0).unwrap_or(&0), 1); // was 2, now 1
        assert_eq!(*rs.by_type().get(&3).unwrap_or(&0), 1);
        assert_eq!(*rs.by_type().get(&4).unwrap_or(&0), 1);
    }

    #[test]
    fn test_malformed_rolls_off() {
        let mut rs = RollingSummary::new(2);
        rs.push(0, true, vec![], None);
        rs.push(0, false, vec![], None);
        assert_eq!(rs.malformed_messages(), 1);
        rs.push(0, false, vec![], None); // rolls off the malformed one
        assert_eq!(rs.malformed_messages(), 0);
    }

    #[test]
    fn test_window_size_zero() {
        let mut rs = RollingSummary::new(0);
        rs.push(0, false, vec![], None);
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
        rs.push(0, false, vec![f1.clone()], None);
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.other_findings, 1);

        rs.push(0, false, vec![f2.clone()], None);
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.other_findings, 2);

        // Push 3rd — oldest findings roll off
        rs.push(0, false, vec![], None);
        let buckets = rs.findings_buckets();
        assert_eq!(buckets.other_findings, 1); // only f2 remains
    }

    #[test]
    fn test_peer_tracking() {
        let pk_a = PeerKey {
            peer_asn: Some(64501),
            peer_ip: Some("10.0.0.1".into()),
            peer_distinguisher: None,
        };
        let pk_b = PeerKey {
            peer_asn: Some(64502),
            peer_ip: Some("10.0.0.2".into()),
            peer_distinguisher: None,
        };
        let pk_c = PeerKey {
            peer_asn: Some(64503),
            peer_ip: Some("10.0.0.3".into()),
            peer_distinguisher: None,
        };

        let mut rs = RollingSummary::new(3);
        rs.push(0, false, vec![], Some(pk_a));
        rs.push(0, false, vec![], Some(pk_b.clone()));
        rs.push(0, false, vec![], Some(pk_c));
        assert_eq!(rs.peers_observed(), 3);

        // 4th message with pk_b — evicts first slot (pk_a, which was unique)
        // peer_counts: pk_b 1->2, pk_c 1, pk_a removed
        rs.push(0, false, vec![], Some(pk_b));
        assert_eq!(rs.peers_observed(), 2);
    }
}
