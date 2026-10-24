//! Jitter buffer for audio RTP packets with sequence reordering and packet loss handling.

use crate::rtp::RtpPacket;
use std::collections::BTreeMap;

pub struct JitterBuffer {
    capacity_packets: usize,
    buffer: BTreeMap<u32, RtpPacket>, // Keyed by unwrapped 32-bit sequence number
    highest_seq: Option<u16>,
    cycles: u32,
    next_playout_seq: Option<u32>,
    target_depth: usize,
    packets_received: u64,
    packets_lost: u64,
    consecutive_underflows: usize,
}

impl JitterBuffer {
    pub fn new(target_depth_packets: usize, capacity_packets: usize) -> Self {
        Self {
            capacity_packets,
            buffer: BTreeMap::new(),
            highest_seq: None,
            cycles: 0,
            next_playout_seq: None,
            target_depth: target_depth_packets,
            packets_received: 0,
            packets_lost: 0,
            consecutive_underflows: 0,
        }
    }

    /// Unwraps 16-bit sequence number to monotonic 32-bit index handling rollover
    pub fn unwrap_sequence(&mut self, seq: u16) -> u32 {
        if let Some(highest) = self.highest_seq {
            let diff = seq.wrapping_sub(highest) as i16;
            if diff > 0 {
                if seq < 0x4000 && highest > 0xC000 {
                    self.cycles += 1;
                }
                self.highest_seq = Some(seq);
                (self.cycles << 16) | (seq as u32)
            } else {
                // Out of order packet behind highest
                let cycle = if highest < 0x4000 && seq > 0xC000 && self.cycles > 0 {
                    self.cycles - 1
                } else {
                    self.cycles
                };
                (cycle << 16) | (seq as u32)
            }
        } else {
            self.highest_seq = Some(seq);
            (self.cycles << 16) | (seq as u32)
        }
    }

    pub fn push(&mut self, packet: RtpPacket) {
        self.packets_received += 1;
        let unwrapped = self.unwrap_sequence(packet.sequence_number);
        if let Some(next) = self.next_playout_seq {
            // Drop packet if it's already older than what we've played
            if unwrapped < next {
                return;
            }
        }

        self.buffer.insert(unwrapped, packet);

        // Enforce capacity bound
        while self.buffer.len() > self.capacity_packets {
            self.buffer.pop_first();
        }

        // Keep playout sequence synchronized with surviving buffer window.
        // If eviction discarded packets ahead of playout, advance next to match the oldest surviving packet.
        if let Some(&first_seq) = self.buffer.keys().next() {
            if let Some(next) = self.next_playout_seq {
                if next < first_seq {
                    self.packets_lost += (first_seq - next) as u64;
                    self.next_playout_seq = Some(first_seq);
                }
            }
        }
    }

    /// Attempts to pop the next packet in sequence.
    /// If the buffer has not reached target_depth yet initially, returns None to allow prebuffering.
    /// If playout has started but the next expected packet is missing, increments packets_lost and returns None (triggering PLC).
    /// If a large gap is detected (e.g. sequence jump or massive packet loss), resynchronizes immediately to the earliest buffered packet.
    pub fn pop(&mut self) -> Option<RtpPacket> {
        if self.next_playout_seq.is_none() {
            if self.buffer.len() < self.target_depth {
                return None; // Prebuffering
            }
            if let Some((&first_seq, _)) = self.buffer.iter().next() {
                self.next_playout_seq = Some(first_seq);
            }
        }

        let next = match self.next_playout_seq {
            Some(seq) => seq,
            None => return None,
        };

        // 1. Direct hit: packet next is present in the buffer
        if let Some(pkt) = self.buffer.remove(&next) {
            self.next_playout_seq = Some(next + 1);
            self.consecutive_underflows = 0;
            return Some(pkt);
        }

        // 2. Packet next is missing.
        // Check if there are packets in buffer ahead of next.
        if let Some((&first_seq, _)) = self.buffer.iter().next() {
            if first_seq > next {
                let gap = first_seq - next;
                // If the gap is larger than target_depth (or 3 packets = 60ms),
                // do not stay in prolonged concealment/silence. Resynchronize immediately!
                if gap > (self.target_depth as u32).max(3) {
                    self.packets_lost += gap as u64;
                    self.next_playout_seq = Some(first_seq + 1);
                    self.consecutive_underflows = 0;
                    return self.buffer.remove(&first_seq);
                }
            }
        } else {
            // Buffer is completely empty (underflow).
            self.consecutive_underflows += 1;
            // After 10 consecutive ticks (200ms) of an empty buffer, reset to prebuffering
            // so we don't drift next_playout_seq during silence or network stalls.
            if self.consecutive_underflows >= 10 {
                self.next_playout_seq = None;
                self.consecutive_underflows = 0;
                return None;
            }
        }

        // 3. Normal packet loss concealment window (gap <= target_depth):
        self.next_playout_seq = Some(next + 1);
        self.packets_lost += 1;
        None
    }

    pub fn buffered_count(&self) -> usize {
        self.buffer.len()
    }

    pub fn packets_received(&self) -> u64 {
        self.packets_received
    }

    pub fn packets_lost(&self) -> u64 {
        self.packets_lost
    }

    pub fn is_prebuffering(&self) -> bool {
        self.next_playout_seq.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_jitter_buffer_reordering() {
        let mut jb = JitterBuffer::new(3, 10);
        let p1 = RtpPacket::new(8, 100, 0, 1, Bytes::from_static(b"a"));
        let p2 = RtpPacket::new(8, 101, 160, 1, Bytes::from_static(b"b"));
        let p3 = RtpPacket::new(8, 102, 320, 1, Bytes::from_static(b"c"));

        // Push out-of-order: 102, 100, 101
        jb.push(p3);
        jb.push(p1);
        jb.push(p2);

        // Prebuffering met (3 packets)
        let out1 = jb.pop().expect("packet 100");
        assert_eq!(out1.sequence_number, 100);

        let out2 = jb.pop().expect("packet 101");
        assert_eq!(out2.sequence_number, 101);

        let out3 = jb.pop().expect("packet 102");
        assert_eq!(out3.sequence_number, 102);

        assert_eq!(jb.pop(), None);
    }

    #[test]
    fn test_jitter_buffer_sequence_rollover() {
        let mut jb = JitterBuffer::new(3, 10);
        let p1 = RtpPacket::new(8, 65534, 0, 1, Bytes::from_static(b"x"));
        let p2 = RtpPacket::new(8, 65535, 160, 1, Bytes::from_static(b"y"));
        let p3 = RtpPacket::new(8, 0, 320, 1, Bytes::from_static(b"z"));
        let p4 = RtpPacket::new(8, 1, 480, 1, Bytes::from_static(b"w"));

        jb.push(p1);
        jb.push(p2);
        jb.push(p3);
        jb.push(p4);

        assert_eq!(jb.pop().expect("seq 65534").sequence_number, 65534);
        assert_eq!(jb.pop().expect("seq 65535").sequence_number, 65535);
        assert_eq!(jb.pop().expect("seq 0").sequence_number, 0);
        assert_eq!(jb.pop().expect("seq 1").sequence_number, 1);
    }

    #[test]
    fn test_jitter_buffer_packet_loss() {
        let mut jb = JitterBuffer::new(2, 10);
        let p100 = RtpPacket::new(8, 100, 0, 1, Bytes::from_static(b"1"));
        let p101 = RtpPacket::new(8, 101, 160, 1, Bytes::from_static(b"2"));
        // Packet 102 is lost!
        let p103 = RtpPacket::new(8, 103, 480, 1, Bytes::from_static(b"4"));

        jb.push(p100);
        jb.push(p101);
        jb.push(p103);

        assert_eq!(jb.pop().expect("packet 100").sequence_number, 100);
        assert_eq!(jb.pop().expect("packet 101").sequence_number, 101);
        // Next expected is 102, which is missing -> pop returns None (triggering PLC), packets_lost incremented
        assert_eq!(jb.pop(), None);
        assert_eq!(jb.packets_lost(), 1);
        // Next expected is 103, which is available in buffer
        assert_eq!(jb.pop().expect("packet 103").sequence_number, 103);
    }

    #[test]
    fn test_jitter_buffer_large_gap_resynchronization() {
        let mut jb = JitterBuffer::new(2, 20);
        let p100 = RtpPacket::new(8, 100, 0, 1, Bytes::from_static(b"1"));
        let p101 = RtpPacket::new(8, 101, 160, 1, Bytes::from_static(b"2"));
        // Sequence jumps by 50 packets (e.g. PBX restart or network re-routing)
        let p151 = RtpPacket::new(8, 151, 8000, 1, Bytes::from_static(b"x"));

        jb.push(p100);
        jb.push(p101);
        jb.push(p151);

        assert_eq!(jb.pop().expect("packet 100").sequence_number, 100);
        assert_eq!(jb.pop().expect("packet 101").sequence_number, 101);

        // Next expected was 102, but buffer contains 151.
        // Large gap (151 - 102 = 49 > 3) should immediately resynchronize to 151 instead of 49 ticks of silence!
        let out = jb.pop().expect("packet 151 resynced");
        assert_eq!(out.sequence_number, 151);
        assert_eq!(jb.packets_lost(), 49);
    }

    #[test]
    fn test_jitter_buffer_eviction_under_backpressure() {
        // Buffer capacity is 5 packets
        let mut jb = JitterBuffer::new(2, 5);
        // Playout starts and pops seq 1 and 2
        jb.push(RtpPacket::new(8, 1, 0, 1, Bytes::from_static(b"1")));
        jb.push(RtpPacket::new(8, 2, 160, 1, Bytes::from_static(b"2")));
        assert_eq!(jb.pop().expect("seq 1").sequence_number, 1);
        assert_eq!(jb.pop().expect("seq 2").sequence_number, 2);

        // Playout is paused (e.g. downstream channel blocked).
        // 10 new packets arrive (seq 3 to 12).
        for seq in 3..=12 {
            jb.push(RtpPacket::new(
                8,
                seq,
                (seq as u32) * 160,
                1,
                Bytes::from_static(b"d"),
            ));
        }

        // Capacity was 5, so packets 3..7 were evicted. Buffer now holds 8..12.
        // When playout resumes, next_playout_seq should NOT be stuck on evicted seq 3!
        let out = jb.pop().expect("resumed packet");
        assert_eq!(out.sequence_number, 8);
        assert_eq!(jb.packets_lost(), 5); // 3, 4, 5, 6, 7 counted as lost
    }

    #[test]
    fn test_jitter_buffer_underflow_resets_prebuffering() {
        let mut jb = JitterBuffer::new(3, 10);
        jb.push(RtpPacket::new(8, 10, 0, 1, Bytes::from_static(b"1")));
        jb.push(RtpPacket::new(8, 11, 160, 1, Bytes::from_static(b"2")));
        jb.push(RtpPacket::new(8, 12, 320, 1, Bytes::from_static(b"3")));

        assert_eq!(jb.pop().expect("seq 10").sequence_number, 10);
        assert_eq!(jb.pop().expect("seq 11").sequence_number, 11);
        assert_eq!(jb.pop().expect("seq 12").sequence_number, 12);

        // Buffer is empty. Simulate 10 ticks of silence/underflow (200ms)
        for _ in 0..10 {
            assert_eq!(jb.pop(), None);
        }

        // Underflow reset to prebuffering mode
        assert!(jb.is_prebuffering());

        // New talk spurt starts with seq 50. Prebuffering requires 3 packets before playout starts.
        jb.push(RtpPacket::new(8, 50, 8000, 1, Bytes::from_static(b"a")));
        assert_eq!(jb.pop(), None); // only 1 packet
        jb.push(RtpPacket::new(8, 51, 8160, 1, Bytes::from_static(b"b")));
        assert_eq!(jb.pop(), None); // only 2 packets
        jb.push(RtpPacket::new(8, 52, 8320, 1, Bytes::from_static(b"c")));

        // Target depth met (3 packets) -> playout starts smoothly from seq 50!
        assert_eq!(jb.pop().expect("seq 50").sequence_number, 50);
        assert_eq!(jb.pop().expect("seq 51").sequence_number, 51);
        assert_eq!(jb.pop().expect("seq 52").sequence_number, 52);
    }
}
