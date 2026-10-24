//! RFC 3550 Real-time Transport Protocol (RTP) packet implementation.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum RtpError {
    #[error("RTP packet too short: expected at least 12 bytes, got {0}")]
    PacketTooShort(usize),
    #[error("Unsupported RTP version: {0}")]
    UnsupportedVersion(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpPacket {
    pub version: u8,
    pub padding: bool,
    pub extension: bool,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: Bytes,
}

impl RtpPacket {
    pub const HEADER_LEN: usize = 12;

    pub fn new(
        payload_type: u8,
        sequence_number: u16,
        timestamp: u32,
        ssrc: u32,
        payload: Bytes,
    ) -> Self {
        Self {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            payload,
        }
    }

    pub fn parse(mut raw: &[u8]) -> Result<Self, RtpError> {
        if raw.len() < Self::HEADER_LEN {
            return Err(RtpError::PacketTooShort(raw.len()));
        }

        let b0 = raw.get_u8();
        let version = (b0 >> 6) & 0x03;
        if version != 2 {
            return Err(RtpError::UnsupportedVersion(version));
        }

        let padding = (b0 & 0x20) != 0;
        let extension = (b0 & 0x10) != 0;
        let csrc_count = (b0 & 0x0F) as usize;

        let b1 = raw.get_u8();
        let marker = (b1 & 0x80) != 0;
        let payload_type = b1 & 0x7F;

        let sequence_number = raw.get_u16();
        let timestamp = raw.get_u32();
        let ssrc = raw.get_u32();

        // Skip CSRC list if present
        let csrc_bytes = csrc_count * 4;
        if raw.len() < csrc_bytes {
            return Err(RtpError::PacketTooShort(raw.len()));
        }
        raw.advance(csrc_bytes);

        // Header extension handling if present
        if extension {
            if raw.len() < 4 {
                return Err(RtpError::PacketTooShort(raw.len()));
            }
            let _ext_id = raw.get_u16();
            let ext_len_words = raw.get_u16() as usize;
            let ext_bytes = ext_len_words * 4;
            if raw.len() < ext_bytes {
                return Err(RtpError::PacketTooShort(raw.len()));
            }
            raw.advance(ext_bytes);
        }

        // Padding handling
        let payload_len = if padding {
            if raw.is_empty() {
                return Err(RtpError::PacketTooShort(raw.len()));
            }
            let pad_count = raw[raw.len() - 1] as usize;
            if pad_count > raw.len() {
                return Err(RtpError::PacketTooShort(raw.len()));
            }
            raw.len() - pad_count
        } else {
            raw.len()
        };

        let payload = Bytes::copy_from_slice(&raw[..payload_len]);

        Ok(Self {
            version,
            padding,
            extension,
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            payload,
        })
    }

    pub fn serialize(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(Self::HEADER_LEN + self.payload.len());

        let mut b0 = (self.version & 0x03) << 6;
        if self.padding {
            b0 |= 0x20;
        }
        if self.extension {
            b0 |= 0x10;
        }
        buf.put_u8(b0);

        let mut b1 = self.payload_type & 0x7F;
        if self.marker {
            b1 |= 0x80;
        }
        buf.put_u8(b1);

        buf.put_u16(self.sequence_number);
        buf.put_u32(self.timestamp);
        buf.put_u32(self.ssrc);
        buf.put_slice(&self.payload);

        buf.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtp_roundtrip() {
        let payload = Bytes::from_static(b"\x80\x81\x82\x83\x84\x85");
        let pkt = RtpPacket::new(8, 1024, 80000, 0x12345678, payload.clone());
        let raw = pkt.serialize();
        assert_eq!(raw.len(), RtpPacket::HEADER_LEN + payload.len());

        let parsed = RtpPacket::parse(&raw).expect("valid parse");
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.payload_type, 8);
        assert_eq!(parsed.sequence_number, 1024);
        assert_eq!(parsed.timestamp, 80000);
        assert_eq!(parsed.ssrc, 0x12345678);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn test_rtp_sequence_and_timestamp_rollover() {
        let payload = Bytes::from_static(b"audio_sample");
        // Sequence number at 16-bit max, timestamp near 32-bit max
        let pkt = RtpPacket::new(8, 0xFFFF, 0xFFFF_FFF0, 0xCAFEBABE, payload.clone());
        let raw = pkt.serialize();
        let parsed = RtpPacket::parse(&raw).expect("valid parse");

        assert_eq!(parsed.sequence_number, 65535);
        assert_eq!(parsed.timestamp, 0xFFFF_FFF0);

        // Advance by 1 packet
        let next_seq = parsed.sequence_number.wrapping_add(1);
        let next_ts = parsed.timestamp.wrapping_add(160);
        let next_pkt = RtpPacket::new(8, next_seq, next_ts, 0xCAFEBABE, payload);
        let next_raw = next_pkt.serialize();
        let next_parsed = RtpPacket::parse(&next_raw).expect("valid parse");

        assert_eq!(next_parsed.sequence_number, 0);
        assert_eq!(next_parsed.timestamp, 144); // 0xFFFF_FFF0 + 160 wraps to 144
    }

    #[test]
    fn test_rtp_marker_bit() {
        let payload = Bytes::from_static(b"marker_test");
        let mut pkt = RtpPacket::new(0, 100, 1000, 1, payload.clone());
        pkt.marker = true;

        let raw = pkt.serialize();
        let parsed = RtpPacket::parse(&raw).expect("valid parse");
        assert!(parsed.marker);
        assert_eq!(parsed.payload_type, 0);
        assert_eq!(parsed.payload, payload);
    }
}
