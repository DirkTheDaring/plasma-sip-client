//! Media stream diagnostics and statistics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaStats {
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub packets_received: u64,
    pub bytes_received: u64,
    pub packets_lost: u64,
    pub plc_frames_concealed: u64,
    pub jitter_estimate_ms: f32,
    pub negotiated_codec: String,
}
