//! Milestone R6: Genuine Randomized Fuzz & Property Testing for RTP and DTMF Parsers.
//! Ensures that 10,000+ malformed, truncated, and corrupt packets never panic or crash the audio pipeline.

use bytes::Bytes;
use sip_media::{DtmfEvent, RtpPacket};

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn next_range(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        let span = (max - min) as u32;
        min + (self.next_u32() % span) as usize
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for b in dest.iter_mut() {
            *b = (self.next_u32() & 0xFF) as u8;
        }
    }
}

#[test]
fn test_rtp_parser_static_and_randomized_fuzz() {
    let edge_cases: [&[u8]; 12] = [
        &[],
        &[0x80],
        &[0x80, 0x08],
        &[0x80, 0x08, 0x00, 0x01],
        &[
            0x80, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
        &[0x00; 12],
        &[0x40; 12],
        &[0xC0; 12],
        &[
            0x90, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
        &[
            0xA0, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x05,
        ],
        &[
            0x8F, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
        &[
            0x80, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0xA0, 0x12, 0x34, 0x56, 0x78, 0xFF,
        ],
    ];

    for (idx, input) in edge_cases.iter().enumerate() {
        let result = std::panic::catch_unwind(|| {
            let _ = RtpPacket::parse(input);
        });
        assert!(result.is_ok(), "Static RTP edge case #{} panicked", idx);
    }

    // 10,000 randomized property fuzzing iterations
    let mut rng = SimpleRng::new(0xCAFEBABE1337);
    let valid_base = RtpPacket::new(8, 100, 16000, 0x12345678, Bytes::from_static(&[0x55; 160]));
    let valid_bytes = valid_base.serialize();

    for iteration in 0..10_000 {
        let mutation_type = rng.next_range(0, 4);
        let data: Vec<u8> = match mutation_type {
            0 => {
                // Bit/byte corruptions on valid packet
                let mut mutated = valid_bytes.to_vec();
                let flips = rng.next_range(1, 8);
                for _ in 0..flips {
                    let pos = rng.next_range(0, mutated.len());
                    mutated[pos] ^= (1 << rng.next_range(0, 8)) as u8;
                }
                mutated
            }
            1 => {
                // Random truncations
                let cut = rng.next_range(0, valid_bytes.len());
                valid_bytes[..cut].to_vec()
            }
            2 => {
                // Pure random noise buffers
                let len = rng.next_range(0, 1500);
                let mut buf = vec![0u8; len];
                rng.fill_bytes(&mut buf);
                buf
            }
            _ => {
                // Manipulating padding count & extension header fields
                let mut mutated = valid_bytes.to_vec();
                mutated[0] |= 0x20; // set padding flag
                let pad_len = rng.next_range(0, 255) as u8;
                mutated.push(pad_len);
                mutated
            }
        };

        let result = std::panic::catch_unwind(|| {
            let _ = RtpPacket::parse(&data);
        });
        assert!(
            result.is_ok(),
            "RTP parser panicked on fuzz iteration #{}",
            iteration
        );
    }
}

#[test]
fn test_dtmf_parser_static_and_randomized_fuzz() {
    let edge_cases: [&[u8]; 5] = [
        &[],
        &[0x05],
        &[0x05, 0x8A],
        &[0x05, 0x8A, 0x00],
        &[0x05, 0x8A, 0x01, 0x00],
    ];

    for (idx, input) in edge_cases.iter().enumerate() {
        let result = std::panic::catch_unwind(|| {
            let _ = DtmfEvent::parse(input);
        });
        assert!(result.is_ok(), "Static DTMF edge case #{} panicked", idx);
    }

    // 10,000 randomized property fuzzing iterations for DTMF
    let mut rng = SimpleRng::new(0xBEEFDEADBEEF);
    for iteration in 0..10_000 {
        let len = rng.next_range(0, 32);
        let mut buf = vec![0u8; len];
        rng.fill_bytes(&mut buf);

        let result = std::panic::catch_unwind(|| {
            let _ = DtmfEvent::parse(&buf);
        });
        assert!(
            result.is_ok(),
            "DTMF parser panicked on fuzz iteration #{}",
            iteration
        );
    }
}
