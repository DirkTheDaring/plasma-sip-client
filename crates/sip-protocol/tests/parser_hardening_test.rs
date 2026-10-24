//! Milestone R6: Genuine Randomized Fuzz & Property Testing for SIP and SDP Parsers.
//! Verifies that 10,000+ malformed, mutated, truncated, and high-entropy inputs never panic or crash.

use sip_protocol::{SdpSession, SipMessage};

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

    fn next_ascii_string(&mut self, len: usize) -> String {
        let mut s = String::with_capacity(len);
        let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \t\r\n:;=<>/.,\"'\\-!@#$%^&*()_+{}[]|";
        for _ in 0..len {
            let idx = (self.next_u32() as usize) % chars.len();
            s.push(chars[idx] as char);
        }
        s
    }
}

#[test]
fn test_sip_parser_static_and_randomized_fuzz() {
    let edge_cases = [
        "",
        "\r\n",
        "\r\n\r\n",
        "NOT A SIP MESSAGE",
        "SIP/2.0",
        "SIP/2.0 ",
        "SIP/2.0 200",
        "INVITE",
        "INVITE sip:bob@example.com",
        "INVITE sip:bob@example.com NOT_SIP/2.0\r\n\r\n",
        "INVITE sip:bob@example.com SIP/2.0\r\nHeaderWithoutColon\r\n\r\n",
        "INVITE sip:bob@example.com SIP/2.0\r\n: MissingKey\r\n\r\n",
        "SIP/2.0 9999999999999999999999999 InvalidStatus\r\n\r\n",
        "SIP/2.0 abc NonNumericStatus\r\n\r\n",
        "INVITE \x00\x7F\u{FFFF} SIP/2.0\r\n\r\n",
        &"A".repeat(100_000), // Huge input boundary
    ];

    for (idx, input) in edge_cases.iter().enumerate() {
        let result = std::panic::catch_unwind(|| {
            let _ = SipMessage::parse(input);
        });
        assert!(result.is_ok(), "Static edge case #{} panicked", idx);
    }

    // 10,000 randomized property fuzzing iterations
    let base_valid = "INVITE sip:bob@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK1234\r\nFrom: <sip:alice@example.com>;tag=abc\r\nTo: <sip:bob@example.com>\r\nCall-ID: fuzz-call-id-999\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n";
    let mut rng = SimpleRng::new(0xDEADBEEFCAFE);

    for iteration in 0..10_000 {
        let mutation_type = rng.next_range(0, 5);
        let input = match mutation_type {
            0 => {
                // Random byte mutation / corruption of valid message
                let mut bytes = base_valid.as_bytes().to_vec();
                let mutations = rng.next_range(1, 10);
                for _ in 0..mutations {
                    let pos = rng.next_range(0, bytes.len());
                    bytes[pos] = (rng.next_u32() & 0xFF) as u8;
                }
                String::from_utf8_lossy(&bytes).to_string()
            }
            1 => {
                // Truncation at random offset
                let truncate_at = rng.next_range(0, base_valid.len());
                base_valid[..truncate_at].to_string()
            }
            2 => {
                // Random ASCII / control character noise
                let len = rng.next_range(0, 1024);
                rng.next_ascii_string(len)
            }
            3 => {
                // Deep header line folding and whitespace noise
                let mut s = "INVITE sip:user@host SIP/2.0\r\nHeader: val".to_string();
                let folds = rng.next_range(1, 50);
                for _ in 0..folds {
                    s.push_str("\r\n continuation_line_token_without_colon");
                }
                s.push_str("\r\n\r\n");
                s
            }
            _ => {
                // Header flood near or past boundaries
                let header_count = rng.next_range(1, 150);
                let mut s = "SIP/2.0 200 OK\r\n".to_string();
                for i in 0..header_count {
                    s.push_str(&format!("X-Fuzz-Header-{}: fuzz_value_{}\r\n", i, i));
                }
                s.push_str("\r\n");
                s
            }
        };

        let result = std::panic::catch_unwind(|| {
            let _ = SipMessage::parse(&input);
        });
        assert!(
            result.is_ok(),
            "SIP parser panicked on fuzz iteration #{}",
            iteration
        );
    }
}

#[test]
fn test_sdp_parser_static_and_randomized_fuzz() {
    let edge_cases = [
        "",
        "v=0",
        "m=audio",
        "m=audio not_a_port",
        "m=audio 5000",
        "m=audio 5000 RTP/AVP",
        "m=video 5000 RTP/AVP 8",
        "c=IN IP4",
        "c=IN IP4 not_an_ip",
        "c=IN IP6 ::1",
        "v=0\r\no=alice 1 1 IN IP4 127.0.0.1\r\ns=Session\r\nc=IN IP4 999.999.999.999\r\nm=audio 8000 RTP/AVP 8",
        &"v=0\r\n".repeat(1000),
    ];

    for (idx, input) in edge_cases.iter().enumerate() {
        let result = std::panic::catch_unwind(|| {
            let _ = SdpSession::parse(input);
        });
        assert!(result.is_ok(), "Static SDP edge case #{} panicked", idx);
    }

    // 10,000 randomized property fuzzing iterations for SDP
    let base_sdp = "v=0\r\no=alice 12345 12345 IN IP4 127.0.0.1\r\ns=Test\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 8 0 101\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
    let mut rng = SimpleRng::new(0xFEEDFACE1234);

    for iteration in 0..10_000 {
        let mutation_type = rng.next_range(0, 4);
        let input = match mutation_type {
            0 => {
                let mut bytes = base_sdp.as_bytes().to_vec();
                let mutations = rng.next_range(1, 8);
                for _ in 0..mutations {
                    let pos = rng.next_range(0, bytes.len());
                    bytes[pos] = (rng.next_u32() & 0xFF) as u8;
                }
                String::from_utf8_lossy(&bytes).to_string()
            }
            1 => {
                let truncate_at = rng.next_range(0, base_sdp.len());
                base_sdp[..truncate_at].to_string()
            }
            2 => {
                let len = rng.next_range(0, 512);
                rng.next_ascii_string(len)
            }
            _ => {
                let lines = rng.next_range(1, 100);
                let mut s = String::new();
                for _ in 0..lines {
                    let field = match rng.next_range(0, 5) {
                        0 => "v=",
                        1 => "m=audio ",
                        2 => "c=IN IP4 ",
                        3 => "a=",
                        _ => "x=",
                    };
                    s.push_str(field);
                    let val_len = rng.next_range(1, 30);
                    let val_str = rng.next_ascii_string(val_len);
                    s.push_str(&val_str);
                    s.push_str("\r\n");
                }
                s
            }
        };

        let result = std::panic::catch_unwind(|| {
            let _ = SdpSession::parse(&input);
        });
        assert!(
            result.is_ok(),
            "SDP parser panicked on fuzz iteration #{}",
            iteration
        );
    }
}
