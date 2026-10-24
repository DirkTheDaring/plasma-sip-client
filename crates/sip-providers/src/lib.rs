//! Telephony provider profiles for FRITZ!Box PBX and Generic SIP.

pub mod fritzbox_tr064;
pub use fritzbox_tr064::*;

use sip_domain::{DialTarget, DomainError, ProviderKind, SipAccount};

pub trait SipProviderProfile: Send + Sync {
    fn provider_kind(&self) -> ProviderKind;
    fn display_name(&self) -> &'static str;
    fn default_registrar(&self) -> &'static str;
    fn default_port(&self) -> u16 {
        5060
    }
    fn validate_account(&self, account: &SipAccount) -> Result<(), DomainError>;
    fn normalize_dial_target(
        &self,
        raw_number: &str,
        account: &SipAccount,
    ) -> Result<DialTarget, DomainError>;
}

/// Provider profile tailored for AVM FRITZ!Box telephony.
#[derive(Debug, Default, Clone)]
pub struct FritzBoxProvider;

impl SipProviderProfile for FritzBoxProvider {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::FritzBox
    }

    fn display_name(&self) -> &'static str {
        "AVM FRITZ!Box"
    }

    fn default_registrar(&self) -> &'static str {
        "fritz.box"
    }

    fn validate_account(&self, account: &SipAccount) -> Result<(), DomainError> {
        account.validate()?;
        if account.username.len() < 3 {
            return Err(DomainError::InvalidAccountConfig(
                "FRITZ!Box telephony username should usually be at least 3 characters (e.g. '620')"
                    .into(),
            ));
        }
        Ok(())
    }

    fn normalize_dial_target(
        &self,
        raw_number: &str,
        account: &SipAccount,
    ) -> Result<DialTarget, DomainError> {
        let trimmed = raw_number.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidDialTarget(
                "Target number cannot be empty".into(),
            ));
        }

        // FRITZ!Box internal extension numbers begin with '**' (e.g. **610, **620, **9)
        let is_internal = trimmed.starts_with("**")
            || (trimmed.chars().all(|c| c.is_ascii_digit()) && trimmed.len() <= 3);

        let sanitized: String = trimmed
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '*' || *c == '#' || *c == '+')
            .collect();

        if sanitized.is_empty() {
            return Err(DomainError::InvalidDialTarget(
                "No valid dial digits found".into(),
            ));
        }

        let sip_uri = format!("sip:{}@{}", sanitized, account.domain);

        Ok(DialTarget {
            raw: raw_number.to_string(),
            normalized_sip_uri: sip_uri,
            display_name: None,
            is_internal_extension: is_internal,
        })
    }
}

/// Generic standards-compliant SIP provider profile.
#[derive(Debug, Default, Clone)]
pub struct GenericSipProvider;

impl SipProviderProfile for GenericSipProvider {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::GenericSip
    }

    fn display_name(&self) -> &'static str {
        "Generic SIP Provider"
    }

    fn default_registrar(&self) -> &'static str {
        ""
    }

    fn validate_account(&self, account: &SipAccount) -> Result<(), DomainError> {
        account.validate()
    }

    fn normalize_dial_target(
        &self,
        raw_number: &str,
        account: &SipAccount,
    ) -> Result<DialTarget, DomainError> {
        let trimmed = raw_number.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidDialTarget(
                "Target cannot be empty".into(),
            ));
        }

        if trimmed.starts_with("sip:") || trimmed.starts_with("sips:") {
            if trimmed
                .chars()
                .any(|c| c.is_control() || c == '\r' || c == '\n' || c.is_whitespace())
            {
                return Err(DomainError::InvalidDialTarget(
                    "SIP URI contains invalid control characters or whitespace".into(),
                ));
            }
            return Ok(DialTarget {
                raw: raw_number.to_string(),
                normalized_sip_uri: trimmed.to_string(),
                display_name: None,
                is_internal_extension: false,
            });
        }

        // Normalize digits
        let sanitized: String = trimmed
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+' || *c == '*' || *c == '#')
            .collect();

        let sip_uri = format!("sip:{}@{}", sanitized, account.domain);

        Ok(DialTarget {
            raw: raw_number.to_string(),
            normalized_sip_uri: sip_uri,
            display_name: None,
            is_internal_extension: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sip_domain::{AccountId, TransportProtocol};

    fn dummy_account() -> SipAccount {
        SipAccount {
            id: AccountId::new(),
            name: "FritzBox".into(),
            username: "620".into(),
            display_name: Some("Desk".into()),
            domain: "fritz.box".into(),
            registrar: "fritz.box".into(),
            registrar_port: 5060,
            transport: TransportProtocol::Udp,
            expires_secs: 300,
            enabled: true,
            is_default: true,
        }
    }

    #[test]
    fn test_fritzbox_dial_normalization() {
        let provider = FritzBoxProvider;
        let account = dummy_account();

        // Internal extension
        let t1 = provider
            .normalize_dial_target("**610", &account)
            .expect("normalize **610");
        assert_eq!(t1.normalized_sip_uri, "sip:**610@fritz.box");
        assert!(t1.is_internal_extension);

        // Short digit extension
        let t2 = provider
            .normalize_dial_target("611", &account)
            .expect("normalize 611");
        assert_eq!(t2.normalized_sip_uri, "sip:611@fritz.box");
        assert!(t2.is_internal_extension);

        // External number (using exact BNetzA listed drama numbers: 0172 9925904 and +49 172 9968532)
        let t3 = provider
            .normalize_dial_target("0172 9925904", &account)
            .expect("normalize external");
        assert_eq!(t3.normalized_sip_uri, "sip:01729925904@fritz.box");
        assert!(!t3.is_internal_extension);

        let t4 = provider
            .normalize_dial_target("+49 172 9968532", &account)
            .expect("normalize international mobile");
        assert_eq!(t4.normalized_sip_uri, "sip:+491729968532@fritz.box");
        assert!(!t4.is_internal_extension);

        // Invalid empty target
        assert!(provider.normalize_dial_target("   ", &account).is_err());
    }

    #[test]
    fn test_generic_sip_dial_normalization() {
        let provider = GenericSipProvider;
        let mut account = dummy_account();
        account.domain = "sip.example.com".into();

        let t1 = provider
            .normalize_dial_target("+49 30 23125000", &account)
            .expect("normalize e164");
        assert_eq!(t1.normalized_sip_uri, "sip:+493023125000@sip.example.com");

        let t2 = provider
            .normalize_dial_target("sip:alice@sip.example.com", &account)
            .expect("direct uri");
        assert_eq!(t2.normalized_sip_uri, "sip:alice@sip.example.com");

        // CRLF / header injection attempt must fail
        assert!(
            provider
                .normalize_dial_target("sip:alice@sip.example.com\r\nInjected: true", &account)
                .is_err()
        );
        assert!(
            provider
                .normalize_dial_target("sip:alice@sip.example.com\nInjected: true", &account)
                .is_err()
        );
        assert!(
            provider
                .normalize_dial_target("sip:alice with spaces", &account)
                .is_err()
        );
    }
}
