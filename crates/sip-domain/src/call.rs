use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use crate::account::AccountId;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallId(pub Uuid);

impl CallId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CallId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    Idle,
    Dialing,
    RingingRemote,
    RingingLocal,
    Active { started_at: DateTime<Utc> },
    Held,
    Terminating { reason: DisconnectReason },
    Terminated { reason: DisconnectReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    LocalHangup,
    RemoteHangup,
    Busy,
    Rejected,
    Declined,
    Cancelled,
    Unreachable,
    Timeout,
    AuthenticationFailed,
    CodecMismatch,
    NetworkError(String),
}

impl fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalHangup => write!(f, "Call ended by local user"),
            Self::RemoteHangup => write!(f, "Call ended by remote party"),
            Self::Busy => write!(f, "Destination busy"),
            Self::Rejected => write!(f, "Call rejected"),
            Self::Declined => write!(f, "Call declined"),
            Self::Cancelled => write!(f, "Call cancelled"),
            Self::Unreachable => write!(f, "Destination unreachable"),
            Self::Timeout => write!(f, "Call timed out"),
            Self::AuthenticationFailed => write!(f, "Authentication failed"),
            Self::CodecMismatch => write!(f, "Codec negotiation failed"),
            Self::NetworkError(msg) => write!(f, "Network error: {}", msg),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCallInfo {
    pub id: CallId,
    pub account_id: AccountId,
    pub direction: CallDirection,
    pub remote_party: String,
    pub state: CallState,
    pub muted: bool,
    pub created_at: DateTime<Utc>,
    pub connected_at: Option<DateTime<Utc>>,
}

impl CallState {
    pub fn transition_to(&self, target: CallState) -> Result<CallState, DomainError> {
        let valid = matches!(
            (self, &target),
            (CallState::Idle, CallState::Dialing)
                | (CallState::Idle, CallState::RingingLocal)
                | (CallState::Dialing, CallState::RingingRemote)
                | (CallState::Dialing, CallState::Active { .. })
                | (CallState::Dialing, CallState::Terminating { .. })
                | (CallState::Dialing, CallState::Terminated { .. })
                | (CallState::RingingRemote, CallState::Active { .. })
                | (CallState::RingingRemote, CallState::Terminating { .. })
                | (CallState::RingingRemote, CallState::Terminated { .. })
                | (CallState::RingingLocal, CallState::Active { .. })
                | (CallState::RingingLocal, CallState::Terminating { .. })
                | (CallState::RingingLocal, CallState::Terminated { .. })
                | (CallState::Active { .. }, CallState::Held)
                | (CallState::Active { .. }, CallState::Active { .. })
                | (CallState::Active { .. }, CallState::Terminating { .. })
                | (CallState::Active { .. }, CallState::Terminated { .. })
                | (CallState::Held, CallState::Active { .. })
                | (CallState::Held, CallState::Held)
                | (CallState::Held, CallState::Terminating { .. })
                | (CallState::Held, CallState::Terminated { .. })
                | (CallState::Terminating { .. }, CallState::Terminated { .. })
        );

        if valid {
            Ok(target)
        } else {
            Err(DomainError::InvalidCallStateTransition {
                from: format!("{:?}", self),
                to: format!("{:?}", target),
            })
        }
    }
}

/// Configurable behavior when declining an incoming call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CallRejectMode {
    /// 603 Decline (RFC 3261 explicit callee decline) - default.
    #[default]
    Decline603,
    /// 486 Busy Here (callee is busy at this station).
    BusyHere486,
    /// 600 Busy Everywhere (all stations / callee busy).
    BusyEverywhere600,
    /// Answer & immediate BYE (forcefully cancels other phones on PBX and terminates caller).
    AnswerAndDrop,
}

impl CallRejectMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Decline603 => "603_decline",
            Self::BusyHere486 => "486_busy_here",
            Self::BusyEverywhere600 => "600_busy_everywhere",
            Self::AnswerAndDrop => "answer_and_drop",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "486" | "busy_here" | "486_busy_here" => Self::BusyHere486,
            "600" | "busy_everywhere" | "600_busy_everywhere" => Self::BusyEverywhere600,
            "drop" | "answer_and_drop" | "answer_drop" => Self::AnswerAndDrop,
            _ => Self::Decline603,
        }
    }
}

