//! Session Description Protocol (SDP, RFC 4566 / RFC 3264).

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum SdpError {
    #[error("Missing or invalid 'c=' connection line")]
    MissingConnectionLine,
    #[error("Missing or invalid 'm=' media line")]
    MissingMediaLine,
    #[error("Invalid port in media line")]
    InvalidPort,
    #[error("Parse error: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaDirection {
    #[default]
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

impl fmt::Display for MediaDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SendRecv => write!(f, "sendrecv"),
            Self::SendOnly => write!(f, "sendonly"),
            Self::RecvOnly => write!(f, "recvonly"),
            Self::Inactive => write!(f, "inactive"),
        }
    }
}

impl From<&str> for MediaDirection {
    fn from(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "sendonly" => Self::SendOnly,
            "recvonly" => Self::RecvOnly,
            "inactive" => Self::Inactive,
            _ => Self::SendRecv,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpSession {
    pub origin_username: String,
    pub session_id: u64,
    pub session_version: u64,
    pub connection_ip: IpAddr,
    pub audio_port: u16,
    pub payload_types: Vec<u8>, // e.g. [8, 0, 101] for PCMA, PCMU, telephone-event
    pub direction: MediaDirection,
    pub rtp_map: HashMap<u8, String>,
}

impl SdpSession {
    pub fn new(local_ip: IpAddr, audio_port: u16, payload_types: Vec<u8>) -> Self {
        let mut rtp_map = HashMap::new();
        for &pt in &payload_types {
            match pt {
                8 => {
                    rtp_map.insert(8, "PCMA/8000".to_string());
                }
                0 => {
                    rtp_map.insert(0, "PCMU/8000".to_string());
                }
                101 => {
                    rtp_map.insert(101, "telephone-event/8000".to_string());
                }
                _ => {}
            }
        }

        Self {
            origin_username: "plasma-sip".into(),
            session_id: 1000,
            session_version: 1,
            connection_ip: local_ip,
            audio_port,
            payload_types,
            direction: MediaDirection::SendRecv,
            rtp_map,
        }
    }

    pub fn with_direction(mut self, direction: MediaDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Returns the negotiated payload type for RFC 4733 DTMF telephony events if present
    pub fn dtmf_payload_type(&self) -> Option<u8> {
        for (&pt, enc) in &self.rtp_map {
            if enc.to_ascii_lowercase().starts_with("telephone-event") {
                return Some(pt);
            }
        }
        if self.payload_types.contains(&101) {
            Some(101)
        } else {
            None
        }
    }

    /// Serializes to standard SDP text according to RFC 4566
    pub fn serialize(&self) -> String {
        let ip_version = match self.connection_ip {
            IpAddr::V4(_) => "IP4",
            IpAddr::V6(_) => "IP6",
        };

        let pt_str = self
            .payload_types
            .iter()
            .map(|pt| pt.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        let mut lines = vec![
            "v=0".to_string(),
            format!(
                "o={} {} {} IN {} {}",
                self.origin_username,
                self.session_id,
                self.session_version,
                ip_version,
                self.connection_ip
            ),
            "s=PlasmaSip".to_string(),
            format!("c=IN {} {}", ip_version, self.connection_ip),
            "t=0 0".to_string(),
            format!("m=audio {} RTP/AVP {}", self.audio_port, pt_str),
        ];

        for &pt in &self.payload_types {
            if let Some(enc) = self.rtp_map.get(&pt) {
                lines.push(format!("a=rtpmap:{} {}", pt, enc));
                if enc.to_ascii_lowercase().starts_with("telephone-event") {
                    lines.push(format!("a=fmtp:{} 0-16", pt));
                }
            } else {
                match pt {
                    8 => lines.push("a=rtpmap:8 PCMA/8000".to_string()),
                    0 => lines.push("a=rtpmap:0 PCMU/8000".to_string()),
                    101 => {
                        lines.push("a=rtpmap:101 telephone-event/8000".to_string());
                        lines.push("a=fmtp:101 0-16".to_string());
                    }
                    other => lines.push(format!("a=rtpmap:{} unknown/8000", other)),
                }
            }
        }
        lines.push(format!("a={}", self.direction));
        lines.push("a=ptime:20".to_string());

        // CRLF line endings required by RFC 4566
        lines.join("\r\n") + "\r\n"
    }

    /// Parses remote SDP offer or answer
    pub fn parse(sdp: &str) -> Result<Self, SdpError> {
        let mut connection_ip = None;
        let mut audio_port = None;
        let mut payload_types = Vec::new();
        let mut direction = MediaDirection::SendRecv;
        let mut rtp_map = HashMap::new();

        for line in sdp.lines() {
            let trimmed = line.trim();
            if let Some(c_line) = trimmed.strip_prefix("c=") {
                // Example: c=IN IP4 192.0.2.1
                let parts: Vec<&str> = c_line.split_whitespace().collect();
                if parts.len() >= 3
                    && let Ok(ip) = parts[2].parse::<IpAddr>()
                {
                    connection_ip = Some(ip);
                }
            } else if let Some(m_line) = trimmed.strip_prefix("m=") {
                // Example: m=audio 7078 RTP/AVP 8 0 101
                let parts: Vec<&str> = m_line.split_whitespace().collect();
                if parts.len() >= 4 && parts[0] == "audio" {
                    let port = parts[1].parse::<u16>().map_err(|_| SdpError::InvalidPort)?;
                    audio_port = Some(port);
                    for pt_str in &parts[3..] {
                        if let Ok(pt) = pt_str.parse::<u8>() {
                            payload_types.push(pt);
                        }
                    }
                }
            } else if let Some(a_line) = trimmed.strip_prefix("a=") {
                let lower = a_line.to_ascii_lowercase();
                if lower == "sendonly" {
                    direction = MediaDirection::SendOnly;
                } else if lower == "recvonly" {
                    direction = MediaDirection::RecvOnly;
                } else if lower == "inactive" {
                    direction = MediaDirection::Inactive;
                } else if lower == "sendrecv" {
                    direction = MediaDirection::SendRecv;
                } else if let Some(rtpmap_body) = trimmed.strip_prefix("a=rtpmap:") {
                    // Example: a=rtpmap:96 telephone-event/8000
                    let mut parts = rtpmap_body.split_whitespace();
                    if let (Some(pt_str), Some(encoding)) = (parts.next(), parts.next())
                        && let Ok(pt) = pt_str.parse::<u8>()
                    {
                        rtp_map.insert(pt, encoding.to_string());
                    }
                }
            }
        }

        let ip = connection_ip.ok_or(SdpError::MissingConnectionLine)?;
        let port = audio_port.ok_or(SdpError::MissingMediaLine)?;

        Ok(Self {
            origin_username: "remote".into(),
            session_id: 0,
            session_version: 0,
            connection_ip: ip,
            audio_port: port,
            payload_types,
            direction,
            rtp_map,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_sdp_roundtrip() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 20));
        let sdp = SdpSession::new(ip, 16384, vec![8, 0, 101]);
        let serialized = sdp.serialize();
        assert!(serialized.contains("m=audio 16384 RTP/AVP 8 0 101"));
        assert!(serialized.contains("c=IN IP4 192.0.2.20"));
        assert!(serialized.contains("a=sendrecv"));

        let parsed = SdpSession::parse(&serialized).expect("valid sdp parse");
        assert_eq!(parsed.connection_ip, ip);
        assert_eq!(parsed.audio_port, 16384);
        assert_eq!(parsed.payload_types, vec![8, 0, 101]);
        assert_eq!(parsed.direction, MediaDirection::SendRecv);
        assert_eq!(parsed.dtmf_payload_type(), Some(101));
    }

    #[test]
    fn test_sdp_dynamic_telephone_event_and_direction() {
        let raw = "v=0\r\n\
c=IN IP4 10.0.0.1\r\n\
m=audio 34560 RTP/AVP 96 8 0\r\n\
a=rtpmap:96 telephone-event/8000\r\n\
a=fmtp:96 0-16\r\n\
a=sendonly\r\n";

        let parsed = SdpSession::parse(raw).expect("parse sdp");
        assert_eq!(parsed.direction, MediaDirection::SendOnly);
        assert_eq!(parsed.payload_types, vec![96, 8, 0]);
        assert_eq!(parsed.dtmf_payload_type(), Some(96));
    }
}
