//! Pure domain models and invariants for Plasma SIP softphone.

pub mod account;
pub mod audio;
pub mod call;
pub mod error;
pub mod provider;

pub use account::*;
pub use audio::*;
pub use call::*;
pub use error::*;
pub use provider::*;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_sip_account_validation() {
        let mut account = SipAccount {
            id: AccountId::new(),
            name: "Test Acc".into(),
            username: "".into(), // Invalid
            display_name: None,
            domain: "fritz.box".into(),
            registrar: "fritz.box".into(),
            registrar_port: 5060,
            transport: TransportProtocol::Udp,
            expires_secs: 300,
            enabled: true,
            is_default: true,
        };

        assert!(account.validate().is_err());
        account.username = "620".into();
        assert!(account.validate().is_ok());

        account.expires_secs = 10; // Too small (< 30)
        assert!(account.validate().is_err());
        account.expires_secs = 3600;
        assert!(account.validate().is_ok());

        account.registrar_port = 0; // Invalid port
        assert!(account.validate().is_err());
        account.registrar_port = 5060;
        assert!(account.validate().is_ok());

        assert_eq!(account.aor(), "sip:620@fritz.box");
        assert_eq!(account.registrar_uri(), "sip:fritz.box");
    }

    #[test]
    fn test_call_state_transitions() {
        let state = CallState::Idle;
        // Valid transitions
        let dialing = state
            .transition_to(CallState::Dialing)
            .expect("Idle -> Dialing");
        let ringing = dialing
            .transition_to(CallState::RingingRemote)
            .expect("Dialing -> RingingRemote");
        let active = ringing
            .transition_to(CallState::Active {
                started_at: Utc::now(),
            })
            .expect("RingingRemote -> Active");
        let terminating = active
            .transition_to(CallState::Terminating {
                reason: DisconnectReason::LocalHangup,
            })
            .expect("Active -> Terminating");
        let terminated = terminating
            .transition_to(CallState::Terminated {
                reason: DisconnectReason::LocalHangup,
            })
            .expect("Terminating -> Terminated");
        assert!(matches!(terminated, CallState::Terminated { .. }));

        // Invalid transitions
        assert!(state
            .transition_to(CallState::Active {
                started_at: Utc::now(),
            })
            .is_err());
    }
}
