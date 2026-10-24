//! RFC 3261 SIP Message representations and builders.

use std::collections::HashMap;
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_SIP_MESSAGE_LEN: usize = 65536;
pub const MAX_SIP_HEADERS: usize = 128;
pub const MAX_HEADER_LINE_LEN: usize = 4096;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum SipParseError {
    #[error("SIP message is empty")]
    EmptyMessage,
    #[error("Message too large: {0} bytes (max 65536)")]
    MessageTooLarge(usize),
    #[error("Too many headers: {0} (max 128)")]
    TooManyHeaders(usize),
    #[error("Malformed start line: {0}")]
    MalformedStartLine(String),
    #[error("Missing mandatory header: {0}")]
    MissingHeader(String),
    #[error("Header parse error: {0}")]
    HeaderError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipMethod {
    Register,
    Invite,
    Ack,
    Cancel,
    Bye,
    Options,
    Other(String),
}

impl fmt::Display for SipMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register => write!(f, "REGISTER"),
            Self::Invite => write!(f, "INVITE"),
            Self::Ack => write!(f, "ACK"),
            Self::Cancel => write!(f, "CANCEL"),
            Self::Bye => write!(f, "BYE"),
            Self::Options => write!(f, "OPTIONS"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

impl From<&str> for SipMethod {
    fn from(s: &str) -> Self {
        match s {
            "REGISTER" => Self::Register,
            "INVITE" => Self::Invite,
            "ACK" => Self::Ack,
            "CANCEL" => Self::Cancel,
            "BYE" => Self::Bye,
            "OPTIONS" => Self::Options,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipRequest {
    pub method: SipMethod,
    pub ruri: String,
    pub headers: HashMap<String, String>,
    pub raw_headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipResponse {
    pub status_code: u16,
    pub reason_phrase: String,
    pub headers: HashMap<String, String>,
    pub raw_headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipMessage {
    Request(SipRequest),
    Response(SipResponse),
}

impl SipMessage {
    pub fn parse(raw: &str) -> Result<Self, SipParseError> {
        if raw.len() > MAX_SIP_MESSAGE_LEN {
            return Err(SipParseError::MessageTooLarge(raw.len()));
        }

        let mut lines = raw.lines();
        let start_line = lines.next().ok_or(SipParseError::EmptyMessage)?.trim_end();
        if start_line.is_empty() {
            return Err(SipParseError::EmptyMessage);
        }

        let mut raw_headers: Vec<(String, String)> = Vec::new();
        let mut headers: HashMap<String, String> = HashMap::new();
        let mut body_lines = Vec::new();
        let mut in_body = false;

        for line in lines {
            if in_body {
                body_lines.push(line);
            } else if line.is_empty() || line == "\r" {
                in_body = true;
            } else {
                if line.len() > MAX_HEADER_LINE_LEN {
                    return Err(SipParseError::HeaderError(
                        "Header line exceeds maximum length".to_string(),
                    ));
                }
                // RFC 3261 Section 7.3.1 Header line folding (SP or HTAB continuation)
                if line.starts_with(' ') || line.starts_with('\t') {
                    if let Some((_, last_val)) = raw_headers.last_mut() {
                        let trimmed = line.trim();
                        last_val.push(' ');
                        last_val.push_str(trimmed);
                        let last_key = raw_headers.last().unwrap().0.clone();
                        if let Some(hval) = headers.get_mut(&last_key) {
                            hval.push(' ');
                            hval.push_str(trimmed);
                        }
                    } else {
                        return Err(SipParseError::HeaderError(
                            "Unexpected folded line at start of headers".to_string(),
                        ));
                    }
                } else if let Some((k, v)) = line.split_once(':') {
                    if raw_headers.len() >= MAX_SIP_HEADERS {
                        return Err(SipParseError::TooManyHeaders(raw_headers.len()));
                    }
                    let key = normalize_header_name(k.trim());
                    let val = v.trim().to_string();
                    raw_headers.push((key.clone(), val.clone()));

                    if key == "Via" {
                        // Top Via kept in primary headers map for backward compatibility
                        headers.entry(key).or_insert(val);
                    } else if key == "Route" || key == "Record-Route" {
                        headers
                            .entry(key)
                            .and_modify(|existing| {
                                existing.push_str(", ");
                                existing.push_str(&val);
                            })
                            .or_insert(val);
                    } else {
                        headers.insert(key, val);
                    }
                }
            }
        }

        let body = body_lines.join("\r\n");

        if start_line.starts_with("SIP/2.0 ") {
            // Response: SIP/2.0 200 OK
            let parts: Vec<&str> = start_line.splitn(3, ' ').collect();
            if parts.len() < 2 {
                return Err(SipParseError::MalformedStartLine(start_line.to_string()));
            }
            let status_code = parts[1]
                .parse::<u16>()
                .map_err(|_| SipParseError::MalformedStartLine(start_line.to_string()))?;
            let reason_phrase = if parts.len() > 2 {
                parts[2].to_string()
            } else {
                "".to_string()
            };

            Ok(SipMessage::Response(SipResponse {
                status_code,
                reason_phrase,
                headers,
                raw_headers,
                body,
            }))
        } else {
            // Request: REGISTER sip:fritz.box SIP/2.0
            let parts: Vec<&str> = start_line.splitn(3, ' ').collect();
            if parts.len() < 3 || parts[2] != "SIP/2.0" {
                return Err(SipParseError::MalformedStartLine(start_line.to_string()));
            }
            let method = SipMethod::from(parts[0]);
            let ruri = parts[1].to_string();

            Ok(SipMessage::Request(SipRequest {
                method,
                ruri,
                headers,
                raw_headers,
                body,
            }))
        }
    }
}

impl SipRequest {
    pub fn new(method: SipMethod, ruri: impl Into<String>) -> Self {
        Self {
            method,
            ruri: ruri.into(),
            headers: HashMap::new(),
            raw_headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn serialize(&self) -> String {
        let clean_ruri = self.ruri.replace(['\r', '\n'], "");
        let mut msg = format!("{} {} SIP/2.0\r\n", self.method, clean_ruri);
        if !self.raw_headers.is_empty() {
            for (k, v) in &self.raw_headers {
                let clean_k = k.replace(['\r', '\n'], "");
                let clean_v = v.replace(['\r', '\n'], "");
                msg.push_str(&format!("{}: {}\r\n", clean_k, clean_v));
            }
        } else {
            for (k, v) in &self.headers {
                let clean_k = k.replace(['\r', '\n'], "");
                let clean_v = v.replace(['\r', '\n'], "");
                msg.push_str(&format!("{}: {}\r\n", clean_k, clean_v));
            }
        }
        msg.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));
        msg.push_str(&self.body);
        msg
    }

    pub fn call_id(&self) -> Option<&str> {
        self.get_header("Call-ID")
    }

    pub fn cseq(&self) -> Option<&str> {
        self.get_header("CSeq")
    }

    pub fn add_header(&mut self, name: &str, value: &str) {
        let norm = normalize_header_name(name);
        self.raw_headers.push((name.to_string(), value.to_string()));
        self.headers.insert(norm, value.to_string());
    }

    pub fn get_header(&self, name: &str) -> Option<&str> {
        let norm = normalize_header_name(name);
        self.raw_headers
            .iter()
            .find(|(k, _)| normalize_header_name(k) == norm)
            .map(|(_, v)| v.as_str())
            .or_else(|| self.headers.get(&norm).map(|s| s.as_str()))
    }

    pub fn get_headers(&self, name: &str) -> Vec<&str> {
        let norm = normalize_header_name(name);
        let from_raw: Vec<&str> = self
            .raw_headers
            .iter()
            .filter(|(k, _)| normalize_header_name(k) == norm)
            .map(|(_, v)| v.as_str())
            .collect();
        if !from_raw.is_empty() {
            from_raw
        } else if let Some(h) = self.headers.get(&norm) {
            h.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_first_via(&self) -> Option<&str> {
        self.get_headers("Via").first().copied()
    }

    pub fn get_all_vias(&self) -> Vec<&str> {
        self.get_headers("Via")
    }

    pub fn get_record_routes(&self) -> Vec<String> {
        let mut routes = Vec::new();
        for r in self.get_headers("Record-Route") {
            for entry in r.split(',') {
                let trimmed = entry.trim();
                if !trimmed.is_empty() {
                    routes.push(trimmed.to_string());
                }
            }
        }
        routes
    }

    pub fn get_routes(&self) -> Vec<String> {
        let mut routes = Vec::new();
        for r in self.get_headers("Route") {
            for entry in r.split(',') {
                let trimmed = entry.trim();
                if !trimmed.is_empty() {
                    routes.push(trimmed.to_string());
                }
            }
        }
        routes
    }

    /// Extracts the caller party information from P-Asserted-Identity, Remote-Party-ID, or From.
    pub fn caller_info(&self) -> SipPartyInfo {
        if let Some(pai) = self.get_header("P-Asserted-Identity") {
            if let Some(info) = SipPartyInfo::parse(pai) {
                return info;
            }
        }
        if let Some(rpi) = self.get_header("Remote-Party-ID") {
            if let Some(info) = SipPartyInfo::parse(rpi) {
                return info;
            }
        }
        if let Some(from) = self.get_header("From") {
            if let Some(info) = SipPartyInfo::parse(from) {
                return info;
            }
        }
        SipPartyInfo {
            number_or_user: "Unknown".into(),
            display_name: None,
            host: None,
            display_string: "Unknown Caller".into(),
        }
    }

    /// Extracts the called / dialed number (callee DID) from P-Called-Party-ID, X-Called-Party-ID, Diversion, or To.
    pub fn called_info(&self) -> Option<SipPartyInfo> {
        if let Some(p_called) = self.get_header("P-Called-Party-ID") {
            if let Some(info) = SipPartyInfo::parse(p_called) {
                return Some(info);
            }
        }
        if let Some(x_called) = self.get_header("X-Called-Party-ID") {
            if let Some(info) = SipPartyInfo::parse(x_called) {
                return Some(info);
            }
        }
        if let Some(div) = self.get_header("Diversion") {
            if let Some(info) = SipPartyInfo::parse(div) {
                return Some(info);
            }
        }
        if let Some(to) = self.get_header("To") {
            if let Some(info) = SipPartyInfo::parse(to) {
                return Some(info);
            }
        }
        None
    }
}

impl SipResponse {
    pub fn new(status_code: u16, reason_phrase: impl Into<String>) -> Self {
        Self {
            status_code,
            reason_phrase: reason_phrase.into(),
            headers: HashMap::new(),
            raw_headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn serialize(&self) -> String {
        let clean_reason = self.reason_phrase.replace(['\r', '\n'], "");
        let mut msg = format!("SIP/2.0 {} {}\r\n", self.status_code, clean_reason);
        if !self.raw_headers.is_empty() {
            for (k, v) in &self.raw_headers {
                let clean_k = k.replace(['\r', '\n'], "");
                let clean_v = v.replace(['\r', '\n'], "");
                msg.push_str(&format!("{}: {}\r\n", clean_k, clean_v));
            }
        } else {
            for (k, v) in &self.headers {
                let clean_k = k.replace(['\r', '\n'], "");
                let clean_v = v.replace(['\r', '\n'], "");
                msg.push_str(&format!("{}: {}\r\n", clean_k, clean_v));
            }
        }
        msg.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));
        msg.push_str(&self.body);
        msg
    }

    pub fn call_id(&self) -> Option<&str> {
        self.get_header("Call-ID")
    }

    pub fn cseq(&self) -> Option<&str> {
        self.get_header("CSeq")
    }

    pub fn add_header(&mut self, name: &str, value: &str) {
        let norm = normalize_header_name(name);
        self.raw_headers.push((name.to_string(), value.to_string()));
        self.headers.insert(norm, value.to_string());
    }

    pub fn get_header(&self, name: &str) -> Option<&str> {
        let norm = normalize_header_name(name);
        self.raw_headers
            .iter()
            .find(|(k, _)| normalize_header_name(k) == norm)
            .map(|(_, v)| v.as_str())
            .or_else(|| self.headers.get(&norm).map(|s| s.as_str()))
    }

    pub fn get_headers(&self, name: &str) -> Vec<&str> {
        let norm = normalize_header_name(name);
        let from_raw: Vec<&str> = self
            .raw_headers
            .iter()
            .filter(|(k, _)| normalize_header_name(k) == norm)
            .map(|(_, v)| v.as_str())
            .collect();
        if !from_raw.is_empty() {
            from_raw
        } else if let Some(h) = self.headers.get(&norm) {
            h.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_first_via(&self) -> Option<&str> {
        self.get_headers("Via").first().copied()
    }

    pub fn get_all_vias(&self) -> Vec<&str> {
        self.get_headers("Via")
    }

    pub fn get_record_routes(&self) -> Vec<String> {
        let mut routes = Vec::new();
        for r in self.get_headers("Record-Route") {
            for entry in r.split(',') {
                let trimmed = entry.trim();
                if !trimmed.is_empty() {
                    routes.push(trimmed.to_string());
                }
            }
        }
        routes
    }

    pub fn get_routes(&self) -> Vec<String> {
        let mut routes = Vec::new();
        for r in self.get_headers("Route") {
            for entry in r.split(',') {
                let trimmed = entry.trim();
                if !trimmed.is_empty() {
                    routes.push(trimmed.to_string());
                }
            }
        }
        routes
    }

    /// Constructs an RFC 3261 response matching an incoming request.
    pub fn create_for_request(
        req: &SipRequest,
        status_code: u16,
        reason_phrase: &str,
        to_tag: Option<&str>,
        contact: Option<&str>,
        body: Option<String>,
    ) -> Self {
        let mut raw_headers = Vec::new();
        let mut headers = HashMap::new();

        // RFC 3261 Section 8.2.6.2: UAS MUST copy all Via headers in exact order
        for via in req.get_headers("Via") {
            raw_headers.push(("Via".to_string(), via.to_string()));
            if !headers.contains_key("Via") {
                headers.insert("Via".into(), via.to_string());
            }
        }

        // RFC 3261 Section 12.1.1: UAS MUST copy all Record-Route headers in exact order
        for rr in req.get_headers("Record-Route") {
            raw_headers.push(("Record-Route".to_string(), rr.to_string()));
            headers
                .entry("Record-Route".into())
                .and_modify(|existing: &mut String| {
                    existing.push_str(", ");
                    existing.push_str(rr);
                })
                .or_insert_with(|| rr.to_string());
        }

        if let Some(from) = req.headers.get("From") {
            raw_headers.push(("From".into(), from.clone()));
            headers.insert("From".into(), from.clone());
        }
        if let Some(to) = req.headers.get("To") {
            let to_header = match to_tag {
                Some(tag) if !to.contains(";tag=") => format!("{};tag={}", to, tag),
                _ => to.clone(),
            };
            raw_headers.push(("To".into(), to_header.clone()));
            headers.insert("To".into(), to_header);
        }
        if let Some(call_id) = req.headers.get("Call-ID") {
            raw_headers.push(("Call-ID".into(), call_id.clone()));
            headers.insert("Call-ID".into(), call_id.clone());
        }
        if let Some(cseq) = req.headers.get("CSeq") {
            raw_headers.push(("CSeq".into(), cseq.clone()));
            headers.insert("CSeq".into(), cseq.clone());
        }
        if let Some(contact_val) = contact {
            raw_headers.push(("Contact".into(), contact_val.to_string()));
            headers.insert("Contact".into(), contact_val.to_string());
        }
        raw_headers.push(("User-Agent".into(), "PlasmaSip/0.1".into()));
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());

        let body_str = body.unwrap_or_default();
        if !body_str.is_empty() {
            raw_headers.push(("Content-Type".into(), "application/sdp".into()));
            headers.insert("Content-Type".into(), "application/sdp".into());
        }

        Self {
            status_code,
            reason_phrase: reason_phrase.to_string(),
            headers,
            raw_headers,
            body: body_str,
        }
    }
}

pub fn generate_branch() -> String {
    format!("z9hG4bK{}", Uuid::new_v4().as_simple())
}

pub fn generate_tag() -> String {
    format!("{:x}", Uuid::new_v4().as_fields().0)
}

fn normalize_header_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "v" | "via" => "Via".into(),
        "f" | "from" => "From".into(),
        "t" | "to" => "To".into(),
        "i" | "call-id" => "Call-ID".into(),
        "cseq" => "CSeq".into(),
        "m" | "contact" => "Contact".into(),
        "l" | "content-length" => "Content-Length".into(),
        "c" | "content-type" => "Content-Type".into(),
        "max-forwards" => "Max-Forwards".into(),
        "user-agent" => "User-Agent".into(),
        "expires" => "Expires".into(),
        "authorization" => "Authorization".into(),
        "www-authenticate" => "WWW-Authenticate".into(),
        "proxy-authenticate" => "Proxy-Authenticate".into(),
        "proxy-authorization" => "Proxy-Authorization".into(),
        "route" => "Route".into(),
        "record-route" => "Record-Route".into(),
        _ => name.to_string(),
    }
}

/// Parsed caller/callee party identity and telephone number representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipPartyInfo {
    /// Clean telephone number or user identifier (e.g. "01729925904", "03023125000", "alice")
    pub number_or_user: String,
    /// Human-readable display name if provided (e.g. "Alice Smith", "Alice")
    pub display_name: Option<String>,
    /// Host or domain if available (e.g. "fritz.box", "192.0.2.1")
    pub host: Option<String>,
    /// Human-facing display string (e.g. "01729925904" or "Alice Smith (01729925904)")
    pub display_string: String,
}

impl SipPartyInfo {
    /// Parses any SIP/Tel address header (From, To, P-Asserted-Identity, Remote-Party-ID, etc.)
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut display_name = None;
        let uri_part: &str;

        if let Some(open_idx) = trimmed.find('<') {
            let prefix = trimmed[..open_idx].trim().trim_matches('"').trim();
            if !prefix.is_empty() {
                display_name = Some(prefix.to_string());
            }
            if let Some(close_idx) = trimmed[open_idx + 1..].find('>') {
                uri_part = &trimmed[open_idx + 1..open_idx + 1 + close_idx];
            } else {
                uri_part = trimmed[open_idx + 1..].trim();
            }
        } else if let (Some(open_p), Some(close_p)) = (trimmed.find('('), trimmed.rfind(')')) {
            if open_p < close_p {
                let prefix = trimmed[..open_p].trim().trim_matches('"').trim();
                if !prefix.is_empty() {
                    display_name = Some(prefix.to_string());
                }
                uri_part = trimmed[open_p + 1..close_p].trim();
            } else {
                uri_part = trimmed.split(';').next().unwrap_or(trimmed).trim();
            }
        } else {
            uri_part = trimmed.split(';').next().unwrap_or(trimmed).trim();
        }

        let clean_uri = uri_part.trim();
        let stripped = if let Some(s) = clean_uri.strip_prefix("sip:") {
            s
        } else if let Some(s) = clean_uri.strip_prefix("sips:") {
            s
        } else if let Some(s) = clean_uri.strip_prefix("tel:") {
            s
        } else {
            clean_uri
        };

        let without_params = stripped.split(';').next().unwrap_or(stripped).trim();

        let (number_or_user, host) = if let Some((user, h)) = without_params.split_once('@') {
            let clean_host = h.split(':').next().unwrap_or(h).trim();
            (user.trim().to_string(), Some(clean_host.to_string()))
        } else {
            (without_params.trim().to_string(), None)
        };

        if number_or_user.is_empty() {
            return None;
        }

        let display_string = match &display_name {
            Some(name) if name.eq_ignore_ascii_case("anonymous") || number_or_user.eq_ignore_ascii_case("anonymous") => {
                "Anonymous".to_string()
            }
            Some(name) if name == &number_or_user => {
                number_or_user.clone()
            }
            Some(name) if name.eq_ignore_ascii_case(&number_or_user) => {
                name.clone()
            }
            Some(name) if !name.is_empty() => {
                format!("{} ({})", name, number_or_user)
            }
            _ => {
                if number_or_user.eq_ignore_ascii_case("anonymous") {
                    "Anonymous".to_string()
                } else {
                    number_or_user.clone()
                }
            }
        };

        Some(Self {
            number_or_user,
            display_name,
            host,
            display_string,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sip_response() {
        let raw = "SIP/2.0 401 Unauthorized\r\nVia: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK123\r\nFrom: <sip:620@fritz.box>;tag=abc\r\nTo: <sip:620@fritz.box>;tag=def\r\nCall-ID: test-call-id\r\nCSeq: 1 REGISTER\r\nWWW-Authenticate: Digest realm=\"fritz.box\", nonce=\"12345\"\r\nContent-Length: 0\r\n\r\n";
        let msg = SipMessage::parse(raw).expect("valid parse");
        if let SipMessage::Response(res) = msg {
            assert_eq!(res.status_code, 401);
            assert_eq!(res.reason_phrase, "Unauthorized");
            assert_eq!(
                res.headers.get("Call-ID").map(|s| s.as_str()),
                Some("test-call-id")
            );
            assert_eq!(
                res.headers.get("CSeq").map(|s| s.as_str()),
                Some("1 REGISTER")
            );
            assert!(res.headers.contains_key("WWW-Authenticate"));
        } else {
            panic!("Expected response");
        }
    }

    #[test]
    fn test_multi_via_headers_preserved_in_order() {
        let raw = "INVITE sip:bob@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP proxy1.example.com:5060;branch=z9hG4bK-p1\r\n\
Via: SIP/2.0/UDP proxy2.example.com:5060;branch=z9hG4bK-p2\r\n\
Via: SIP/2.0/UDP client.example.com:5060;branch=z9hG4bK-client\r\n\
From: <sip:alice@example.com>;tag=123\r\n\
To: <sip:bob@example.com>\r\n\
Call-ID: test-multi-via\r\n\
CSeq: 1 INVITE\r\n\
Content-Length: 0\r\n\r\n";

        let msg = SipMessage::parse(raw).expect("valid parse");
        let req = match msg {
            SipMessage::Request(r) => r,
            _ => panic!("Expected request"),
        };

        // All 3 Vias preserved in exact order
        let vias = req.get_all_vias();
        assert_eq!(vias.len(), 3);
        assert_eq!(
            vias[0],
            "SIP/2.0/UDP proxy1.example.com:5060;branch=z9hG4bK-p1"
        );
        assert_eq!(
            vias[1],
            "SIP/2.0/UDP proxy2.example.com:5060;branch=z9hG4bK-p2"
        );
        assert_eq!(
            vias[2],
            "SIP/2.0/UDP client.example.com:5060;branch=z9hG4bK-client"
        );

        // Top via helper returns the first
        assert_eq!(req.get_first_via(), Some(vias[0]));
        // Backward-compatible headers map returns the top Via
        assert_eq!(req.headers.get("Via").map(|s| s.as_str()), Some(vias[0]));

        // Serialization retains all 3 Vias in order
        let serialized = req.serialize();
        assert!(
            serialized.contains("Via: SIP/2.0/UDP proxy1.example.com:5060;branch=z9hG4bK-p1\r\n")
        );
        assert!(
            serialized.contains("Via: SIP/2.0/UDP proxy2.example.com:5060;branch=z9hG4bK-p2\r\n")
        );
        assert!(serialized
            .contains("Via: SIP/2.0/UDP client.example.com:5060;branch=z9hG4bK-client\r\n"));
    }

    #[test]
    fn test_multi_record_routes_and_routes() {
        let raw = "SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP client:5060;branch=z9hG4bK1\r\n\
Record-Route: <sip:proxy1.example.com;lr>\r\n\
Record-Route: <sip:proxy2.example.com;lr>\r\n\
From: <sip:alice@example.com>;tag=123\r\n\
To: <sip:bob@example.com>;tag=456\r\n\
Call-ID: test-rr\r\n\
CSeq: 1 INVITE\r\n\
Content-Length: 0\r\n\r\n";

        let msg = SipMessage::parse(raw).expect("valid parse");
        let resp = match msg {
            SipMessage::Response(r) => r,
            _ => panic!("Expected response"),
        };

        let rrs = resp.get_record_routes();
        assert_eq!(rrs.len(), 2);
        assert_eq!(rrs[0], "<sip:proxy1.example.com;lr>");
        assert_eq!(rrs[1], "<sip:proxy2.example.com;lr>");
    }

    #[test]
    fn test_header_line_folding_rfc3261() {
        let raw = "INVITE sip:bob@example.com SIP/2.0\r\nVia: SIP/2.0/UDP client:5060;branch=z9hG4bK1\r\nSubject: This is an extraordinarily\r\n long subject header line\r\n\twith tab continuation\r\nFrom: <sip:alice@example.com>;tag=123\r\nTo: <sip:bob@example.com>\r\nCall-ID: test-fold\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n";

        let msg = SipMessage::parse(raw).expect("valid parse");
        let req = match msg {
            SipMessage::Request(r) => r,
            _ => panic!("Expected request"),
        };

        let subject = req.get_header("Subject").expect("Subject header");
        assert_eq!(
            subject,
            "This is an extraordinarily long subject header line with tab continuation"
        );
    }

    #[test]
    fn test_message_size_and_header_limits() {
        // Exceeding MAX_SIP_MESSAGE_LEN (64KB)
        let huge_body = "x".repeat(MAX_SIP_MESSAGE_LEN + 10);
        let huge_msg = format!("INVITE sip:bob@example.com SIP/2.0\r\n\r\n{}", huge_body);
        assert!(matches!(
            SipMessage::parse(&huge_msg),
            Err(SipParseError::MessageTooLarge(_))
        ));

        // Exceeding MAX_SIP_HEADERS (128)
        let mut msg_with_many_headers = "INVITE sip:bob@example.com SIP/2.0\r\n".to_string();
        for i in 0..135 {
            msg_with_many_headers.push_str(&format!("X-Header-{}: value\r\n", i));
        }
        msg_with_many_headers.push_str("\r\n");
        assert!(matches!(
            SipMessage::parse(&msg_with_many_headers),
            Err(SipParseError::TooManyHeaders(_))
        ));
    }

    #[test]
    fn test_create_for_request_copies_all_vias_and_record_routes() {
        let raw = "INVITE sip:bob@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP proxy1:5060;branch=z9hG4bK1\r\n\
Via: SIP/2.0/UDP client:5060;branch=z9hG4bK2\r\n\
Record-Route: <sip:proxy1:5060;lr>\r\n\
From: <sip:alice@example.com>;tag=tag1\r\n\
To: <sip:bob@example.com>\r\n\
Call-ID: test-create-resp\r\n\
CSeq: 100 INVITE\r\n\
Content-Length: 0\r\n\r\n";

        let req = match SipMessage::parse(raw).unwrap() {
            SipMessage::Request(r) => r,
            _ => panic!(),
        };

        let resp = SipResponse::create_for_request(
            &req,
            180,
            "Ringing",
            Some("tag2"),
            Some("<sip:bob@192.168.1.50:5060>"),
            None,
        );

        assert_eq!(resp.status_code, 180);
        assert_eq!(resp.get_all_vias().len(), 2);
        assert_eq!(
            resp.get_all_vias()[0],
            "SIP/2.0/UDP proxy1:5060;branch=z9hG4bK1"
        );
        assert_eq!(
            resp.get_all_vias()[1],
            "SIP/2.0/UDP client:5060;branch=z9hG4bK2"
        );
        assert_eq!(resp.get_record_routes().len(), 1);
        assert_eq!(resp.get_record_routes()[0], "<sip:proxy1:5060;lr>");
        assert_eq!(
            resp.headers.get("To").unwrap(),
            "<sip:bob@example.com>;tag=tag2"
        );
    }

    #[test]
    fn test_crlf_injection_prevention_on_serialize() {
        let mut req = SipRequest::new(
            SipMethod::Invite,
            "sip:target@example.com\r\nInjected: evil",
        );
        req.headers
            .insert("Subject".into(), "Normal\r\nInjectedHeader: evil".into());
        req.raw_headers
            .push(("X-Custom\r\n".into(), "Value\r\nInjected: 123".into()));

        let serialized = req.serialize();
        // The serialized message must have exactly one \r\n\r\n separating headers from body
        let header_body_splits: Vec<&str> = serialized.split("\r\n\r\n").collect();
        assert_eq!(header_body_splits.len(), 2);

        // Header section should not contain injected lines
        let header_lines: Vec<&str> = header_body_splits[0].split("\r\n").collect();
        for line in &header_lines {
            assert!(!line.starts_with("Injected:"));
            assert!(!line.starts_with("InjectedHeader:"));
        }
    }

    #[test]
    fn test_sip_party_info_parsing() {
        // FRITZ!Box real incoming call format (using verified BNetzA drama number)
        let p1 = SipPartyInfo::parse("<sip:01729925904@fritz.box>;tag=FB9A3852ABAA095D").unwrap();
        assert_eq!(p1.number_or_user, "01729925904");
        assert_eq!(p1.display_name, None);
        assert_eq!(p1.host.as_deref(), Some("fritz.box"));
        assert_eq!(p1.display_string, "01729925904");

        // Display name + SIP URI with tag
        let p2 = SipPartyInfo::parse("\"Alice Smith\" <sip:01729968532@fritz.box>;tag=959B70C1").unwrap();
        assert_eq!(p2.number_or_user, "01729968532");
        assert_eq!(p2.display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(p2.display_string, "Alice Smith (01729968532)");

        // Display name is same as number
        let p3 = SipPartyInfo::parse("\"01729925904\" <sip:01729925904@fritz.box>").unwrap();
        assert_eq!(p3.number_or_user, "01729925904");
        assert_eq!(p3.display_string, "01729925904");

        // Tel URI
        let p4 = SipPartyInfo::parse("<tel:+491729925904>;tag=123").unwrap();
        assert_eq!(p4.number_or_user, "+491729925904");
        assert_eq!(p4.display_string, "+491729925904");

        // Bare number (BNetzA reserved Berlin drama number)
        let p5 = SipPartyInfo::parse("03023125000").unwrap();
        assert_eq!(p5.number_or_user, "03023125000");
        assert_eq!(p5.display_string, "03023125000");

        // Bare SIP URI with port (RFC 5737 TEST-NET-1 IP)
        let p6 = SipPartyInfo::parse("sip:01729925904@192.0.2.1:5060").unwrap();
        assert_eq!(p6.number_or_user, "01729925904");
        assert_eq!(p6.host.as_deref(), Some("192.0.2.1"));
        assert_eq!(p6.display_string, "01729925904");

        // Anonymous
        let p7 = SipPartyInfo::parse("\"Anonymous\" <sip:anonymous@anonymous.invalid>").unwrap();
        assert_eq!(p7.number_or_user, "anonymous");
        assert_eq!(p7.display_string, "Anonymous");

        // Display name with parentheses number
        let p8 = SipPartyInfo::parse("Alice (01729968532)").unwrap();
        assert_eq!(p8.number_or_user, "01729968532");
        assert_eq!(p8.display_name.as_deref(), Some("Alice"));
        assert_eq!(p8.display_string, "Alice (01729968532)");
    }

    #[test]
    fn test_sip_request_caller_and_called_info() {
        let mut req = SipRequest::new(SipMethod::Invite, "sip:alice-smith@192.0.2.235:5060");
        req.headers.insert("From".into(), "<sip:01729925904@fritz.box>;tag=FB9A3852".into());
        req.headers.insert("To".into(), "<sip:alice-smith@192.0.2.235>".into());
        req.headers.insert("P-Called-Party-ID".into(), "<sip:03023125000@fritz.box>".into());

        let caller = req.caller_info();
        assert_eq!(caller.number_or_user, "01729925904");
        assert_eq!(caller.display_string, "01729925904");

        let called = req.called_info().unwrap();
        assert_eq!(called.number_or_user, "03023125000");
        assert_eq!(called.display_string, "03023125000");
    }
}
