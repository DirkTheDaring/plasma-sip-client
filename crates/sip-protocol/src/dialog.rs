//! RFC 3261 Section 12 Dialog State Machine.

use std::collections::HashMap;
use thiserror::Error;

use crate::message::{SipMethod, SipRequest, SipResponse, generate_branch};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum DialogError {
    #[error("Missing To tag in provisional/final response")]
    MissingToTag,
    #[error("Missing Call-ID header")]
    MissingCallId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DialogId {
    pub call_id: String,
    pub local_tag: String,
    pub remote_tag: String,
}

#[derive(Debug, Clone)]
pub struct SipDialog {
    pub id: DialogId,
    pub local_cseq: u32,
    pub remote_cseq: u32,
    pub local_uri: String,
    pub remote_uri: String,
    pub remote_target: String,
    pub route_set: Vec<String>,
}

impl SipDialog {
    pub fn new_from_outgoing(
        initial_req: &SipRequest,
        response_2xx: &SipResponse,
    ) -> Result<Self, DialogError> {
        let call_id = initial_req
            .headers
            .get("Call-ID")
            .ok_or(DialogError::MissingCallId)?
            .clone();

        let from = initial_req
            .headers
            .get("From")
            .unwrap_or(&"".to_string())
            .clone();
        let to = response_2xx
            .headers
            .get("To")
            .unwrap_or(&"".to_string())
            .clone();

        let local_tag = extract_tag(&from).unwrap_or_default();
        let remote_tag = extract_tag(&to).ok_or(DialogError::MissingToTag)?;

        // CSeq in dialog MUST match the 2xx response CSeq number (RFC 3261 Section 12.1.2)
        let local_cseq = response_2xx
            .cseq()
            .and_then(|c| c.split_whitespace().next())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or_else(|| {
                initial_req
                    .cseq()
                    .and_then(|c| c.split_whitespace().next())
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(1)
            });

        let remote_target = response_2xx
            .headers
            .get("Contact")
            .map(|c| extract_target_uri(c))
            .unwrap_or_else(|| initial_req.ruri.clone());

        // Parse Record-Route in reverse order for UAC (RFC 3261 Section 12.1.2)
        let mut route_set = response_2xx.get_record_routes();
        route_set.reverse();

        Ok(Self {
            id: DialogId {
                call_id,
                local_tag,
                remote_tag,
            },
            local_cseq,
            remote_cseq: 0,
            local_uri: from,
            remote_uri: to,
            remote_target,
            route_set,
        })
    }

    /// Establishes a SIP dialog on the UAS side from an incoming INVITE and our 2xx response.
    pub fn new_from_incoming(
        initial_req: &SipRequest,
        response_2xx: &SipResponse,
    ) -> Result<Self, DialogError> {
        let call_id = initial_req
            .headers
            .get("Call-ID")
            .ok_or(DialogError::MissingCallId)?
            .clone();

        let from = initial_req
            .headers
            .get("From")
            .unwrap_or(&"".to_string())
            .clone();
        let to = response_2xx
            .headers
            .get("To")
            .unwrap_or(&"".to_string())
            .clone();

        let remote_tag = extract_tag(&from).unwrap_or_default();
        let local_tag = extract_tag(&to).ok_or(DialogError::MissingToTag)?;

        let remote_cseq = initial_req
            .cseq()
            .and_then(|c| c.split_whitespace().next())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);

        let remote_target = initial_req
            .headers
            .get("Contact")
            .map(|c| extract_target_uri(c))
            .unwrap_or_else(|| initial_req.ruri.clone());

        // UAS preserves Record-Route order as received (RFC 3261 Section 12.1.1)
        let route_set = initial_req.get_record_routes();

        Ok(Self {
            id: DialogId {
                call_id,
                local_tag,
                remote_tag,
            },
            local_cseq: 1,
            remote_cseq,
            local_uri: to,
            remote_uri: from,
            remote_target,
            route_set,
        })
    }

    /// Builds an in-dialog ACK request for 200 OK to INVITE (RFC 3261 Section 13.2.2.4)
    pub fn create_ack(&self, local_via: &str) -> SipRequest {
        let mut headers: HashMap<String, String> = HashMap::new();
        headers.insert(
            "Via".into(),
            format!("{};branch={}", local_via, generate_branch()),
        );
        headers.insert("Max-Forwards".into(), "70".into());
        headers.insert("From".into(), self.local_uri.clone());
        headers.insert("To".into(), self.remote_uri.clone());
        headers.insert("Call-ID".into(), self.id.call_id.clone());
        headers.insert("CSeq".into(), format!("{} ACK", self.local_cseq));
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());
        if !self.route_set.is_empty() {
            headers.insert("Route".into(), self.route_set.join(", "));
        }

        let raw_headers: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        SipRequest {
            method: SipMethod::Ack,
            ruri: self.remote_target.clone(),
            headers,
            raw_headers,
            body: String::new(),
        }
    }

    /// Builds an in-dialog BYE request to terminate the call (RFC 3261 Section 15.1.1)
    pub fn create_bye(&mut self, local_via: &str) -> SipRequest {
        self.local_cseq += 1;
        let mut headers: HashMap<String, String> = HashMap::new();
        headers.insert(
            "Via".into(),
            format!("{};branch={}", local_via, generate_branch()),
        );
        headers.insert("Max-Forwards".into(), "70".into());
        headers.insert("From".into(), self.local_uri.clone());
        headers.insert("To".into(), self.remote_uri.clone());
        headers.insert("Call-ID".into(), self.id.call_id.clone());
        headers.insert("CSeq".into(), format!("{} BYE", self.local_cseq));
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());
        if !self.route_set.is_empty() {
            headers.insert("Route".into(), self.route_set.join(", "));
        }

        let raw_headers: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        SipRequest {
            method: SipMethod::Bye,
            ruri: self.remote_target.clone(),
            headers,
            raw_headers,
            body: String::new(),
        }
    }

    /// Builds an in-dialog re-INVITE request (RFC 3261 Section 14) for session modification (e.g. Hold/Resume).
    pub fn create_reinvite(
        &mut self,
        local_via: &str,
        contact: &str,
        sdp_body: &str,
    ) -> SipRequest {
        self.local_cseq += 1;
        let mut headers: HashMap<String, String> = HashMap::new();
        headers.insert(
            "Via".into(),
            format!("{};branch={}", local_via, generate_branch()),
        );
        headers.insert("Max-Forwards".into(), "70".into());
        headers.insert("From".into(), self.local_uri.clone());
        headers.insert("To".into(), self.remote_uri.clone());
        headers.insert("Call-ID".into(), self.id.call_id.clone());
        headers.insert("CSeq".into(), format!("{} INVITE", self.local_cseq));
        headers.insert("Contact".into(), contact.to_string());
        headers.insert("Content-Type".into(), "application/sdp".into());
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());
        if !self.route_set.is_empty() {
            headers.insert("Route".into(), self.route_set.join(", "));
        }

        let raw_headers: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        SipRequest {
            method: SipMethod::Invite,
            ruri: self.remote_target.clone(),
            headers,
            raw_headers,
            body: sdp_body.to_string(),
        }
    }

    /// Checks if an incoming SIP request belongs to this active dialog (RFC 3261 Section 12.2.2)
    pub fn matches_incoming_request(&self, req: &SipRequest) -> bool {
        let call_id = match req.headers.get("Call-ID") {
            Some(c) => c,
            None => return false,
        };
        if call_id != &self.id.call_id {
            return false;
        }

        let from_tag = req.headers.get("From").and_then(|f| extract_tag(f));
        let to_tag = req.headers.get("To").and_then(|t| extract_tag(t));

        // For an incoming in-dialog request (sent from remote UAS to local UAC):
        // Remote sent it, so From tag is remote_tag, and To tag is local_tag.
        match (from_tag, to_tag) {
            (Some(f), Some(t)) => f == self.id.remote_tag && t == self.id.local_tag,
            (Some(f), None) => f == self.id.remote_tag && self.id.local_tag.is_empty(),
            _ => false,
        }
    }

    /// Creates an RFC 3261 compliant response to an incoming in-dialog request (e.g. 200 OK to BYE)
    pub fn create_response_to_request(
        &self,
        req: &SipRequest,
        status_code: u16,
        reason: &str,
    ) -> SipResponse {
        SipResponse::create_for_request(
            req,
            status_code,
            reason,
            Some(&self.id.local_tag),
            None,
            None,
        )
    }

    /// Creates an RFC 3261 compliant response to an incoming in-dialog request with custom body (e.g. 200 OK with SDP for re-INVITE)
    pub fn create_response_with_body(
        &self,
        req: &SipRequest,
        status_code: u16,
        reason: &str,
        content_type: &str,
        body: &str,
        contact: Option<&str>,
    ) -> SipResponse {
        let mut resp = SipResponse::create_for_request(
            req,
            status_code,
            reason,
            Some(&self.id.local_tag),
            contact,
            Some(body.to_string()),
        );
        if !content_type.is_empty() {
            resp.headers
                .insert("Content-Type".into(), content_type.to_string());
            if let Some(pos) = resp
                .raw_headers
                .iter()
                .position(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
            {
                resp.raw_headers[pos] = ("Content-Type".to_string(), content_type.to_string());
            } else {
                resp.raw_headers
                    .push(("Content-Type".to_string(), content_type.to_string()));
            }
        }
        resp
    }
}

pub fn extract_tag(header: &str) -> Option<String> {
    for part in header.split(';') {
        let trimmed = part.trim();
        if let Some(tag) = trimmed.strip_prefix("tag=") {
            return Some(tag.to_string());
        }
    }
    None
}

pub fn extract_target_uri(target: &str) -> String {
    let trimmed = target.trim();
    if let Some(start) = trimmed.find('<')
        && let Some(end) = trimmed[start + 1..].find('>')
    {
        return trimmed[start + 1..start + 1 + end].trim().to_string();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialog_cseq_and_route_set() {
        let mut req_headers = HashMap::new();
        req_headers.insert("Call-ID".into(), "test-call-123".into());
        req_headers.insert(
            "From".into(),
            "<sip:alice@example.com>;tag=localtag1".into(),
        );
        req_headers.insert("To".into(), "<sip:bob@example.com>".into());
        req_headers.insert("CSeq".into(), "1 INVITE".into());
        req_headers.insert("Contact".into(), "<sip:alice@192.168.1.10:5060>".into());

        let req = SipRequest {
            method: SipMethod::Invite,
            ruri: "sip:bob@example.com".into(),
            headers: req_headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let mut resp_headers = HashMap::new();
        resp_headers.insert("Call-ID".into(), "test-call-123".into());
        resp_headers.insert(
            "From".into(),
            "<sip:alice@example.com>;tag=localtag1".into(),
        );
        resp_headers.insert("To".into(), "<sip:bob@example.com>;tag=remotetag2".into());
        resp_headers.insert("CSeq".into(), "2 INVITE".into()); // Challenged or updated CSeq
        resp_headers.insert("Contact".into(), "<sip:bob@192.168.1.20:5060>".into());
        resp_headers.insert(
            "Record-Route".into(),
            "<sip:p1.example.com;lr>, <sip:p2.example.com;lr>".into(),
        );

        let resp = SipResponse {
            status_code: 200,
            reason_phrase: "OK".into(),
            headers: resp_headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let mut dialog = SipDialog::new_from_outgoing(&req, &resp).expect("create dialog");
        assert_eq!(dialog.id.call_id, "test-call-123");
        assert_eq!(dialog.id.local_tag, "localtag1");
        assert_eq!(dialog.id.remote_tag, "remotetag2");
        assert_eq!(dialog.local_cseq, 2);
        assert_eq!(dialog.remote_target, "sip:bob@192.168.1.20:5060");
        // Reversed route set
        assert_eq!(
            dialog.route_set,
            vec!["<sip:p2.example.com;lr>", "<sip:p1.example.com;lr>"]
        );

        // Verify ACK headers
        let ack = dialog.create_ack("SIP/2.0/UDP 192.168.1.10:5060");
        assert_eq!(ack.headers.get("CSeq").unwrap(), "2 ACK");
        assert_eq!(
            ack.headers.get("Route").unwrap(),
            "<sip:p2.example.com;lr>, <sip:p1.example.com;lr>"
        );

        // Verify BYE headers
        let bye = dialog.create_bye("SIP/2.0/UDP 192.168.1.10:5060");
        assert_eq!(bye.headers.get("CSeq").unwrap(), "3 BYE");
        assert_eq!(
            bye.headers.get("Route").unwrap(),
            "<sip:p2.example.com;lr>, <sip:p1.example.com;lr>"
        );
    }

    #[test]
    fn test_dialog_incoming_bye_and_response() {
        let mut req_headers = HashMap::new();
        req_headers.insert("Call-ID".into(), "call-999".into());
        req_headers.insert(
            "From".into(),
            "<sip:alice@example.com>;tag=local-aaa".into(),
        );
        req_headers.insert("To".into(), "<sip:bob@example.com>".into());
        req_headers.insert("CSeq".into(), "1 INVITE".into());

        let req = SipRequest {
            method: SipMethod::Invite,
            ruri: "sip:bob@example.com".into(),
            headers: req_headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let mut resp_headers = HashMap::new();
        resp_headers.insert("Call-ID".into(), "call-999".into());
        resp_headers.insert(
            "From".into(),
            "<sip:alice@example.com>;tag=local-aaa".into(),
        );
        resp_headers.insert("To".into(), "<sip:bob@example.com>;tag=remote-bbb".into());
        resp_headers.insert("CSeq".into(), "1 INVITE".into());

        let resp = SipResponse {
            status_code: 200,
            reason_phrase: "OK".into(),
            headers: resp_headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let dialog = SipDialog::new_from_outgoing(&req, &resp).expect("create dialog");

        // Incoming BYE from remote
        let mut bye_headers = HashMap::new();
        bye_headers.insert("Call-ID".into(), "call-999".into());
        bye_headers.insert("From".into(), "<sip:bob@example.com>;tag=remote-bbb".into());
        bye_headers.insert("To".into(), "<sip:alice@example.com>;tag=local-aaa".into());
        bye_headers.insert("CSeq".into(), "10 BYE".into());
        bye_headers.insert(
            "Via".into(),
            "SIP/2.0/UDP 192.168.1.20:5060;branch=z9hG4bKbye1".into(),
        );

        let bye_req = SipRequest {
            method: SipMethod::Bye,
            ruri: "sip:alice@192.168.1.10:5060".into(),
            headers: bye_headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        assert!(dialog.matches_incoming_request(&bye_req));

        let bye_resp = dialog.create_response_to_request(&bye_req, 200, "OK");
        assert_eq!(bye_resp.status_code, 200);
        assert_eq!(bye_resp.headers.get("CSeq").unwrap(), "10 BYE");
        assert_eq!(
            bye_resp.headers.get("Via").unwrap(),
            "SIP/2.0/UDP 192.168.1.20:5060;branch=z9hG4bKbye1"
        );
    }
}
