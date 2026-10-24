use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub Uuid);

impl AccountId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    #[default]
    Udp,
    Tcp,
    Tls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationState {
    #[default]
    Unregistered,
    Registering,
    Registered {
        expires_at_epoch_secs: u64,
    },
    Failed {
        reason: String,
        retry_in_secs: Option<u32>,
    },
    Unregistering,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipAccount {
    pub id: AccountId,
    pub name: String,
    pub username: String,
    pub display_name: Option<String>,
    pub domain: String,
    pub registrar: String,
    pub registrar_port: u16,
    pub transport: TransportProtocol,
    pub expires_secs: u32,
    pub enabled: bool,
    pub is_default: bool,
}

impl SipAccount {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.username.trim().is_empty() {
            return Err(DomainError::InvalidAccountConfig(
                "Username cannot be empty".into(),
            ));
        }
        if self.domain.trim().is_empty() {
            return Err(DomainError::InvalidAccountConfig(
                "Domain cannot be empty".into(),
            ));
        }
        if self.registrar.trim().is_empty() {
            return Err(DomainError::InvalidAccountConfig(
                "Registrar cannot be empty".into(),
            ));
        }
        if self.registrar_port == 0 {
            return Err(DomainError::InvalidAccountConfig(
                "Registrar port cannot be 0".into(),
            ));
        }
        if self.expires_secs < 30 || self.expires_secs > 86400 {
            return Err(DomainError::InvalidAccountConfig(
                "Registration expiry must be between 30 and 86400 seconds".into(),
            ));
        }
        Ok(())
    }

    pub fn aor(&self) -> String {
        format!("sip:{}@{}", self.username, self.domain)
    }

    pub fn registrar_uri(&self) -> String {
        if self.registrar_port == 5060 {
            format!("sip:{}", self.registrar)
        } else {
            format!("sip:{}:{}", self.registrar, self.registrar_port)
        }
    }
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretPassword(pub String);

impl fmt::Debug for SecretPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED SECRET]")
    }
}
