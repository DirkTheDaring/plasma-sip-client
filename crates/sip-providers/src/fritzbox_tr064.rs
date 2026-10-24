//! AVM FRITZ!Box TR-064 (X_AVM-DE_OnTel:1) Phonebook Sync Client.
//! Communicates with the local router over UPnP/TR-064 SOAP with Digest Authentication.

use sip_auth::{DigestAuthorizer, DigestChallenge, DigestResponseParams};
use sip_domain::{Contact, ContactNumber, ContactSource, NumberType};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{debug, info};

#[derive(Error, Debug)]
pub enum Tr064Error {
    #[error("Network I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("HTTP protocol error: {0}")]
    Http(String),
    #[error("SOAP / XML parsing error: {0}")]
    Xml(String),
    #[error("Timeout connecting to FRITZ!Box: {0}")]
    Timeout(String),
}

/// Client for AVM FRITZ!Box TR-064 SOAP interface.
#[derive(Debug, Clone)]
pub struct FritzBoxTr064Client {
    pub host: String,
    pub port: u16,
    pub timeout_duration: Duration,
}

impl FritzBoxTr064Client {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port: if port == 0 { 49000 } else { port },
            timeout_duration: Duration::from_secs(5),
        }
    }

    /// Fetches all contacts from the specified FRITZ!Box phonebook (default ID 0).
    pub async fn fetch_phonebook(
        &self,
        username: &str,
        password: &str,
        phonebook_id: u32,
    ) -> Result<Vec<Contact>, Tr064Error> {
        info!(
            "Querying FRITZ!Box TR-064 phonebook {} at {}:{}...",
            phonebook_id, self.host, self.port
        );

        let soap_path = "/upnp/control/x_contact";
        let soap_action = "urn:dslforum-org:service:X_AVM-DE_OnTel:1#GetPhonebook";
        let soap_body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:GetPhonebook xmlns:u="urn:dslforum-org:service:X_AVM-DE_OnTel:1">
      <NewPhonebookID>{}</NewPhonebookID>
    </u:GetPhonebook>
  </s:Body>
</s:Envelope>"#,
            phonebook_id
        );

        // 1. Send SOAP GetPhonebook request (with Digest auth handshake if needed)
        let response_body = self
            .send_soap_request(soap_path, soap_action, &soap_body, username, password)
            .await?;

        // 2. The response either contains NewPhonebookURL or direct XML
        if let Some(url_str) = extract_tag_value(&response_body, "NewPhonebookURL") {
            debug!("FRITZ!Box returned Phonebook download URL: {}", url_str);
            let path_and_query = if let Some(idx) = url_str.find("/phonebook") {
                &url_str[idx..]
            } else {
                &url_str
            };

            let xml_content = self
                .send_http_get(path_and_query, username, password)
                .await?;
            parse_fritzbox_phonebook_xml(&xml_content)
        } else {
            // Direct XML response
            parse_fritzbox_phonebook_xml(&response_body)
        }
    }

    async fn send_soap_request(
        &self,
        path: &str,
        action: &str,
        body: &str,
        username: &str,
        password: &str,
    ) -> Result<String, Tr064Error> {
        // First try unauthenticated request to get Digest challenge
        let (status, headers, resp_body) = self.execute_http_post(path, action, body, None).await?;

        if status == 200 {
            return Ok(resp_body);
        }

        if status == 401 {
            let auth_header = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
                .map(|(_, v)| v.as_str())
                .ok_or_else(|| Tr064Error::Auth("401 without WWW-Authenticate header".into()))?;

            let challenge = DigestChallenge::parse(auth_header)
                .map_err(|e| Tr064Error::Auth(format!("Challenge parse failed: {}", e)))?;

            let cnonce = format!("{:08x}", rand_u32());
            let auth_params = DigestResponseParams {
                username,
                password,
                realm: &challenge.realm,
                method: "POST",
                digest_uri: path,
                nonce: &challenge.nonce,
                nc: 1,
                cnonce: &cnonce,
                qop: challenge.qop.as_deref(),
                algorithm: Some(&challenge.algorithm),
            };

            let digest_response = DigestAuthorizer::compute_response(&auth_params);
            let auth_header_val = format!(
                r#"Digest username="{}", realm="{}", nonce="{}", uri="{}", algorithm={}, response="{}"{}"#,
                username,
                challenge.realm,
                challenge.nonce,
                path,
                challenge.algorithm,
                digest_response,
                challenge
                    .qop
                    .as_deref()
                    .map(|q| format!(r#", qop="{}", nc=00000001, cnonce="{}""#, q, cnonce))
                    .unwrap_or_default()
            );

            let (status2, _, resp_body2) = self
                .execute_http_post(path, action, body, Some(&auth_header_val))
                .await?;

            if status2 == 200 {
                return Ok(resp_body2);
            } else {
                return Err(Tr064Error::Auth(format!(
                    "Authenticated request rejected with HTTP {}",
                    status2
                )));
            }
        }

        Err(Tr064Error::Http(format!(
            "SOAP request failed with HTTP {}",
            status
        )))
    }

    async fn send_http_get(
        &self,
        path: &str,
        username: &str,
        password: &str,
    ) -> Result<String, Tr064Error> {
        let (status, headers, resp_body) = self.execute_http_get(path, None).await?;
        if status == 200 {
            return Ok(resp_body);
        }

        if status == 401 {
            let auth_header = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
                .map(|(_, v)| v.as_str())
                .ok_or_else(|| Tr064Error::Auth("401 without WWW-Authenticate header".into()))?;

            let challenge = DigestChallenge::parse(auth_header)
                .map_err(|e| Tr064Error::Auth(format!("Challenge parse failed: {}", e)))?;

            let cnonce = format!("{:08x}", rand_u32());
            let auth_params = DigestResponseParams {
                username,
                password,
                realm: &challenge.realm,
                method: "GET",
                digest_uri: path,
                nonce: &challenge.nonce,
                nc: 1,
                cnonce: &cnonce,
                qop: challenge.qop.as_deref(),
                algorithm: Some(&challenge.algorithm),
            };

            let digest_response = DigestAuthorizer::compute_response(&auth_params);
            let auth_header_val = format!(
                r#"Digest username="{}", realm="{}", nonce="{}", uri="{}", algorithm={}, response="{}"{}"#,
                username,
                challenge.realm,
                challenge.nonce,
                path,
                challenge.algorithm,
                digest_response,
                challenge
                    .qop
                    .as_deref()
                    .map(|q| format!(r#", qop="{}", nc=00000001, cnonce="{}""#, q, cnonce))
                    .unwrap_or_default()
            );

            let (status2, _, resp_body2) =
                self.execute_http_get(path, Some(&auth_header_val)).await?;

            if status2 == 200 {
                return Ok(resp_body2);
            } else {
                return Err(Tr064Error::Auth(format!(
                    "Authenticated GET rejected with HTTP {}",
                    status2
                )));
            }
        }

        Err(Tr064Error::Http(format!(
            "HTTP GET {} failed with status {}",
            path, status
        )))
    }

    async fn execute_http_post(
        &self,
        path: &str,
        action: &str,
        body: &str,
        auth_header: Option<&str>,
    ) -> Result<(u16, Vec<(String, String)>, String), Tr064Error> {
        let addr = format!("{}:{}", self.host, self.port);
        let connect_fut = TcpStream::connect(&addr);
        let mut stream = timeout(self.timeout_duration, connect_fut)
            .await
            .map_err(|_| Tr064Error::Timeout(addr.clone()))??;

        let mut req = format!(
            "POST {} HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: Plasma-SIP/1.0\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}\"\r\nContent-Length: {}\r\nConnection: close\r\n",
            path,
            self.host,
            self.port,
            action,
            body.len()
        );

        if let Some(auth) = auth_header {
            req.push_str(&format!("Authorization: {}\r\n", auth));
        }
        req.push_str("\r\n");
        req.push_str(body);

        stream.write_all(req.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;

        parse_http_response(&response)
    }

    async fn execute_http_get(
        &self,
        path: &str,
        auth_header: Option<&str>,
    ) -> Result<(u16, Vec<(String, String)>, String), Tr064Error> {
        let addr = format!("{}:{}", self.host, self.port);
        let connect_fut = TcpStream::connect(&addr);
        let mut stream = timeout(self.timeout_duration, connect_fut)
            .await
            .map_err(|_| Tr064Error::Timeout(addr.clone()))??;

        let mut req = format!(
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: Plasma-SIP/1.0\r\nConnection: close\r\n",
            path, self.host, self.port
        );

        if let Some(auth) = auth_header {
            req.push_str(&format!("Authorization: {}\r\n", auth));
        }
        req.push_str("\r\n");

        stream.write_all(req.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;

        parse_http_response(&response)
    }
}

fn rand_u32() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    now ^ 0x5a5a5a5a
}

pub type HttpRawResponse = (u16, Vec<(String, String)>, String);

fn parse_http_response(raw: &[u8]) -> Result<HttpRawResponse, Tr064Error> {
    let text = String::from_utf8_lossy(raw);
    let mut parts = text.splitn(2, "\r\n\r\n");
    let header_part = parts.next().unwrap_or("");
    let body_part = parts.next().unwrap_or("").to_string();

    let mut lines = header_part.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| Tr064Error::Http("Empty response from server".into()))?;

    let mut status_tokens = status_line.split_whitespace();
    let _http_ver = status_tokens.next();
    let status_code: u16 = status_tokens
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Tr064Error::Http(format!("Invalid HTTP status line: {}", status_line)))?;

    let mut headers = Vec::new();
    for line in lines {
        if let Some(colon) = line.find(':') {
            let k = line[..colon].trim().to_string();
            let v = line[colon + 1..].trim().to_string();
            headers.push((k, v));
        }
    }

    Ok((status_code, headers, body_part))
}

pub fn extract_tag_value(xml: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{}>", tag);
    let close_tag = format!("</{}>", tag);

    if let Some(start) = xml.find(&open_tag) {
        let content_start = start + open_tag.len();
        if let Some(end) = xml[content_start..].find(&close_tag) {
            return Some(xml[content_start..content_start + end].trim().to_string());
        }
    }
    None
}

/// Parses standard AVM FRITZ!Box phonebook XML into domain `Contact` structs.
pub fn parse_fritzbox_phonebook_xml(xml: &str) -> Result<Vec<Contact>, Tr064Error> {
    let mut contacts = Vec::new();
    let contact_tag_open = "<contact>";
    let contact_tag_close = "</contact>";

    let mut search_from = 0;
    while let Some(start) = xml[search_from..].find(contact_tag_open) {
        let abs_start = search_from + start + contact_tag_open.len();
        if let Some(end) = xml[abs_start..].find(contact_tag_close) {
            let block = &xml[abs_start..abs_start + end];
            search_from = abs_start + end + contact_tag_close.len();

            let real_name = extract_tag_value(block, "realName").unwrap_or_default();
            if real_name.trim().is_empty() {
                continue;
            }

            let unique_id = extract_tag_value(block, "uniqueid");
            let mut contact = Contact::new(
                real_name.trim(),
                ContactSource::FritzBox,
                unique_id.map(|s| format!("fb_{}", s)),
            );

            // Parse numbers within <telephony>...</telephony>
            let mut num_search = 0;
            while let Some(n_start) = block[num_search..].find("<number") {
                let abs_n_start = num_search + n_start;
                if let Some(n_tag_end) = block[abs_n_start..].find('>') {
                    let header_str = &block[abs_n_start..abs_n_start + n_tag_end + 1];
                    let val_start = abs_n_start + n_tag_end + 1;
                    if let Some(val_end) = block[val_start..].find("</number>") {
                        let raw_number = block[val_start..val_start + val_end].trim().to_string();
                        num_search = val_start + val_end + "</number>".len();

                        if !raw_number.is_empty() {
                            let number_type = parse_attr_value(header_str, "type")
                                .map(|s| NumberType::from(s.as_str()))
                                .unwrap_or(NumberType::Home);

                            let quickdial = parse_attr_value(header_str, "quickdial");
                            let vanity = parse_attr_value(header_str, "vanity");

                            contact.add_number(ContactNumber::new(
                                raw_number,
                                number_type,
                                quickdial,
                                vanity,
                            ));
                        }
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            if !contact.numbers.is_empty() {
                contacts.push(contact);
            }
        } else {
            break;
        }
    }

    Ok(contacts)
}

fn parse_attr_value(tag_header: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    if let Some(start) = tag_header.find(&needle) {
        let val_start = start + needle.len();
        if let Some(end) = tag_header[val_start..].find('"') {
            let val = tag_header[val_start..val_start + end].trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fritzbox_phonebook_xml_valid() {
        let sample_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<phonebooks>
  <phonebook name="Telefonbuch">
    <contact>
      <category>0</category>
      <person>
        <realName>Alice Schmidt</realName>
      </person>
      <telephony nid="3">
        <number type="home" prio="1" id="0" quickdial="**610">0301234567</number>
        <number type="mobile" prio="0" id="1" vanity="ALICE">+491729925904</number>
        <number type="work" prio="0" id="2">03099887766</number>
      </telephony>
      <services />
      <setup />
      <features doorphone="0" />
      <mod_time>1672531199</mod_time>
      <uniqueid>42</uniqueid>
    </contact>
    <contact>
      <person>
        <realName>Küche (Handset 2)</realName>
      </person>
      <telephony>
        <number type="internal" quickdial="**611">**611</number>
      </telephony>
      <uniqueid>43</uniqueid>
    </contact>
  </phonebook>
</phonebooks>"#;

        let contacts = parse_fritzbox_phonebook_xml(sample_xml).expect("parse valid xml");
        assert_eq!(contacts.len(), 2);

        // Contact 1: Alice Schmidt
        assert_eq!(contacts[0].display_name, "Alice Schmidt");
        assert_eq!(contacts[0].source, ContactSource::FritzBox);
        assert_eq!(contacts[0].fritzbox_unique_id.as_deref(), Some("fb_42"));
        assert_eq!(contacts[0].numbers.len(), 3);
        assert_eq!(contacts[0].numbers[0].raw_number, "0301234567");
        assert_eq!(contacts[0].numbers[0].number_type, NumberType::Home);
        assert_eq!(contacts[0].numbers[0].quickdial.as_deref(), Some("**610"));

        assert_eq!(contacts[0].numbers[1].raw_number, "+491729925904");
        assert_eq!(contacts[0].numbers[1].normalized_digits, "+491729925904");
        assert_eq!(contacts[0].numbers[1].number_type, NumberType::Mobile);
        assert_eq!(contacts[0].numbers[1].vanity.as_deref(), Some("ALICE"));

        // Contact 2: Internal extension
        assert_eq!(contacts[1].display_name, "Küche (Handset 2)");
        assert_eq!(contacts[1].numbers.len(), 1);
        assert_eq!(contacts[1].numbers[0].raw_number, "**611");
        assert_eq!(contacts[1].numbers[0].normalized_digits, "**611");
        assert_eq!(contacts[1].numbers[0].number_type, NumberType::Internal);
    }

    #[test]
    fn test_extract_tag_value() {
        let xml = "<foo><NewPhonebookURL>http://192.168.178.1:49000/phonebook.xml?id=0</NewPhonebookURL></foo>";
        let val = extract_tag_value(xml, "NewPhonebookURL");
        assert_eq!(
            val.as_deref(),
            Some("http://192.168.178.1:49000/phonebook.xml?id=0")
        );
    }
}
