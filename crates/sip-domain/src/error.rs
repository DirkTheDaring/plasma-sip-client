use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("Invalid SIP URI: {0}")]
    InvalidSipUri(String),

    #[error("Invalid account configuration: {0}")]
    InvalidAccountConfig(String),

    #[error("Invalid phone number or dial target: {0}")]
    InvalidDialTarget(String),

    #[error("Call transition error: cannot transition from {from:?} to {to:?}")]
    InvalidCallStateTransition { from: String, to: String },

    #[error("Account {0} not found")]
    AccountNotFound(String),

    #[error("Call {0} not found")]
    CallNotFound(String),

    #[error("Audio device error: {0}")]
    AudioDeviceError(String),
}
