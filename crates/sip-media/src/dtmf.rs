//! RFC 4733 RTP Payload for DTMF Digits and Telephony Tones.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum DtmfError {
    #[error("Invalid DTMF character: {0}")]
    InvalidChar(char),
    #[error("Payload too short for RFC 4733 event")]
    PayloadTooShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DtmfEvent {
    pub event_id: u8, // 0-9: digits, 10: *, 11: #, 12-15: A-D
    pub end_of_event: bool,
    pub volume: u8,    // 0 to 63 (-dBm0)
    pub duration: u16, // in timestamp units (e.g. 800 for 100ms at 8kHz)
}

impl DtmfEvent {
    pub fn from_char(c: char, end_of_event: bool, duration: u16) -> Result<Self, DtmfError> {
        let event_id = match c {
            '0'..='9' => c as u8 - b'0',
            '*' => 10,
            '#' => 11,
            'A' | 'a' => 12,
            'B' | 'b' => 13,
            'C' | 'c' => 14,
            'D' | 'd' => 15,
            _ => return Err(DtmfError::InvalidChar(c)),
        };

        Ok(Self {
            event_id,
            end_of_event,
            volume: 10, // Default moderate volume
            duration,
        })
    }

    pub fn serialize(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u8(self.event_id);
        let b1 = if self.end_of_event { 0x80 } else { 0x00 } | (self.volume & 0x3F);
        buf.put_u8(b1);
        buf.put_u16(self.duration);
        buf.freeze()
    }

    pub fn parse(mut raw: &[u8]) -> Result<Self, DtmfError> {
        if raw.len() < 4 {
            return Err(DtmfError::PayloadTooShort);
        }
        let event_id = raw.get_u8();
        let b1 = raw.get_u8();
        let end_of_event = (b1 & 0x80) != 0;
        let volume = b1 & 0x3F;
        let duration = raw.get_u16();

        Ok(Self {
            event_id,
            end_of_event,
            volume,
            duration,
        })
    }
}
