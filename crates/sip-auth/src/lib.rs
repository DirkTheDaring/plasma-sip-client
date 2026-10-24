//! SIP Digest Authentication (RFC 2617 / RFC 7616 / RFC 3261 Section 22).

use md5::{Digest as Md5Digest, Md5};
use sha2::{Digest as Sha256Digest, Sha256};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum AuthError {
    #[error("Missing realm in challenge")]
    MissingRealm,
    #[error("Missing nonce in challenge")]
    MissingNonce,
    #[error("Unsupported algorithm: {0}")]
    UnsupportedAlgorithm(String),
    #[error("Failed to parse challenge header: {0}")]
    ParseError(String),
}

/// A parsed challenge from a 401 Unauthorized (WWW-Authenticate) or 407 Proxy Authentication Required (Proxy-Authenticate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub stale: bool,
    pub algorithm: String,   // "MD5", "MD5-sess", "SHA-256", "SHA-256-sess"
    pub qop: Option<String>, // e.g. "auth"
}

impl DigestChallenge {
    /// Parse a WWW-Authenticate or Proxy-Authenticate header value:
    /// Example: Digest realm="fritz.box", nonce="12345678", algorithm=MD5, qop="auth"
    pub fn parse(header: &str) -> Result<Self, AuthError> {
        let trimmed = header.trim();
        let stripped = if let Some(rest) = trimmed.strip_prefix("Digest ") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("digest ") {
            rest
        } else {
            trimmed
        };

        let mut params = HashMap::new();
        for part in split_header_params(stripped) {
            let mut kv = part.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                let key = k.trim().to_lowercase();
                let val = v.trim().trim_matches('"').to_string();
                params.insert(key, val);
            }
        }

        let realm = params.remove("realm").ok_or(AuthError::MissingRealm)?;
        let nonce = params.remove("nonce").ok_or(AuthError::MissingNonce)?;
        let opaque = params.remove("opaque");
        let stale = params
            .get("stale")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let algorithm = params
            .remove("algorithm")
            .unwrap_or_else(|| "MD5".to_string());
        let qop = params.remove("qop");

        if !algorithm.eq_ignore_ascii_case("MD5")
            && !algorithm.eq_ignore_ascii_case("MD5-sess")
            && !algorithm.eq_ignore_ascii_case("SHA-256")
            && !algorithm.eq_ignore_ascii_case("SHA-256-sess")
        {
            return Err(AuthError::UnsupportedAlgorithm(algorithm));
        }

        Ok(Self {
            realm,
            nonce,
            opaque,
            stale,
            algorithm,
            qop,
        })
    }
}

pub struct DigestResponseParams<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub realm: &'a str,
    pub method: &'a str,
    pub digest_uri: &'a str,
    pub nonce: &'a str,
    pub nc: u32,
    pub cnonce: &'a str,
    pub qop: Option<&'a str>,
    pub algorithm: Option<&'a str>,
}

fn hash_data(algorithm: &str, data: &[u8]) -> String {
    if algorithm.eq_ignore_ascii_case("SHA-256") || algorithm.eq_ignore_ascii_case("SHA-256-sess") {
        let mut hasher = Sha256::new();
        Sha256Digest::update(&mut hasher, data);
        format!("{:x}", hasher.finalize())
    } else {
        let mut hasher = Md5::new();
        Md5Digest::update(&mut hasher, data);
        format!("{:x}", hasher.finalize())
    }
}

/// Generates an Authorization header value for a request.
pub struct DigestAuthorizer;

impl DigestAuthorizer {
    pub fn compute_response(params: &DigestResponseParams<'_>) -> String {
        let algo = params.algorithm.unwrap_or("MD5");

        // HA1 calculation
        let ha1 =
            if algo.eq_ignore_ascii_case("MD5-sess") || algo.eq_ignore_ascii_case("SHA-256-sess") {
                let ha1_base = hash_data(
                    algo,
                    format!("{}:{}:{}", params.username, params.realm, params.password).as_bytes(),
                );
                hash_data(
                    algo,
                    format!("{}:{}:{}", ha1_base, params.nonce, params.cnonce).as_bytes(),
                )
            } else {
                hash_data(
                    algo,
                    format!("{}:{}:{}", params.username, params.realm, params.password).as_bytes(),
                )
            };

        // HA2 calculation
        let ha2 = hash_data(
            algo,
            format!("{}:{}", params.method, params.digest_uri).as_bytes(),
        );

        // Response calculation
        if let Some(q) = params.qop {
            let nc_hex = format!("{:08x}", params.nc);
            hash_data(
                algo,
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    ha1, params.nonce, nc_hex, params.cnonce, q, ha2
                )
                .as_bytes(),
            )
        } else {
            hash_data(algo, format!("{}:{}:{}", ha1, params.nonce, ha2).as_bytes())
        }
    }

    pub fn build_authorization_header(
        username: &str,
        password: &str,
        challenge: &DigestChallenge,
        method: &str,
        digest_uri: &str,
        nc: u32,
    ) -> String {
        let qop = challenge.qop.as_deref().and_then(|q| {
            if q.split(',').any(|part| part.trim() == "auth") {
                Some("auth")
            } else if q.split(',').any(|part| part.trim() == "auth-int") {
                Some("auth-int")
            } else {
                None
            }
        });

        let cnonce = format!("{:x}", Uuid::new_v4().as_simple());
        let params = DigestResponseParams {
            username,
            password,
            realm: &challenge.realm,
            method,
            digest_uri,
            nonce: &challenge.nonce,
            nc,
            cnonce: &cnonce,
            qop,
            algorithm: Some(&challenge.algorithm),
        };

        let response = Self::compute_response(&params);

        let mut header = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\", algorithm={}",
            username, challenge.realm, challenge.nonce, digest_uri, response, challenge.algorithm
        );

        if let Some(q) = qop {
            let nc_hex = format!("{:08x}", nc);
            header.push_str(&format!(
                ", qop=\"{}\", nc={}, cnonce=\"{}\"",
                q, nc_hex, cnonce
            ));
        }

        if let Some(ref opaque) = challenge.opaque {
            header.push_str(&format!(", opaque=\"{}\"", opaque));
        }

        header
    }
}

fn split_header_params(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in s.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if c == ',' && !in_quotes {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
            current.clear();
        } else {
            current.push(c);
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_string());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fritzbox_challenge() {
        let header =
            r#"Digest realm="fritz.box", nonce="B502C6374EE9E8F3", algorithm=MD5, qop="auth""#;
        let challenge = DigestChallenge::parse(header).expect("valid parse");
        assert_eq!(challenge.realm, "fritz.box");
        assert_eq!(challenge.nonce, "B502C6374EE9E8F3");
        assert_eq!(challenge.algorithm, "MD5");
        assert_eq!(challenge.qop, Some("auth".to_string()));
    }

    #[test]
    fn test_digest_calc_rfc2617_vectors() {
        // RFC 2617 standard test vector check
        let params = DigestResponseParams {
            username: "MGRR",
            password: "secret",
            realm: "testrealm@host.com",
            method: "GET",
            digest_uri: "/dir/index.html",
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            nc: 1,
            cnonce: "0a4f113b",
            qop: Some("auth"),
            algorithm: None,
        };
        let resp = DigestAuthorizer::compute_response(&params);
        assert_eq!(resp, "991716209a9e3c0a9281e862157f14bc");
    }

    #[test]
    fn test_parse_challenge_multi_qop_and_quoted_header() {
        let header =
            r#"Digest realm="fritz.box", nonce="C0FFEE1234", algorithm=MD5, qop="auth,auth-int""#;
        let challenge = DigestChallenge::parse(header).expect("parse multi-qop");
        assert_eq!(challenge.qop, Some("auth,auth-int".to_string()));

        let auth_hdr = DigestAuthorizer::build_authorization_header(
            "620",
            "secret",
            &challenge,
            "REGISTER",
            "sip:fritz.box",
            1,
        );
        assert!(auth_hdr.contains("qop=\"auth\""));
        assert!(!auth_hdr.contains("auth-int"));
    }

    #[test]
    fn test_digest_calc_rfc7616_sha256_vector() {
        let params = DigestResponseParams {
            username: "Mufasa",
            password: "Circle Of Life",
            realm: "http-auth@example.org",
            method: "GET",
            digest_uri: "/dir/index.html",
            nonce: "7ypf/xlj9XXdaM3VJqVaNVuxtfUdflkoTZFdEc3NaQYK",
            nc: 1,
            cnonce: "f2/wE4q74E6zIJEtWaHKaf5wv/H5Qzzcxvf0durafFs",
            qop: Some("auth"),
            algorithm: Some("SHA-256"),
        };
        let resp = DigestAuthorizer::compute_response(&params);
        assert_eq!(
            resp,
            "723f1ab5aba6d1342cba19d71cf926a980b3aace128cd644839ed1b560d88e69"
        );

        let challenge = DigestChallenge {
            realm: "http-auth@example.org".into(),
            nonce: "7ypf/xlj9XXdaM3VJqVaNVuxtfUdflkoTZFdEc3NaQYK".into(),
            opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".into()),
            stale: false,
            algorithm: "SHA-256".into(),
            qop: Some("auth".into()),
        };

        let header = DigestAuthorizer::build_authorization_header(
            "Mufasa",
            "Circle Of Life",
            &challenge,
            "GET",
            "/dir/index.html",
            1,
        );
        assert!(header.contains("algorithm=SHA-256"));
        assert!(header.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
    }
}
