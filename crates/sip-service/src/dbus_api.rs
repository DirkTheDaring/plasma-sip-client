//! D-Bus Service Interface for org.kde.PlasmaSip.

use std::sync::Arc;
use uuid::Uuid;
use zbus::interface;

use sip_audio::{list_input_devices, list_output_devices};
use sip_domain::{AccountId, AudioConfig, CallId, CallRejectMode, SipAccount};
use sip_storage::StorageManager;

use crate::account_manager::AccountManager;
use crate::call_manager::CallManager;

pub struct PlasmaSipDbusService {
    pub account_manager: Arc<AccountManager>,
    pub call_manager: Arc<CallManager>,
    pub storage: Arc<StorageManager>,
}

#[interface(name = "org.kde.PlasmaSip")]
impl PlasmaSipDbusService {
    /// Diagnostic ping method
    async fn ping(&self) -> String {
        "pong".into()
    }

    /// List all configured accounts as a JSON string
    async fn list_accounts(&self) -> String {
        match self.account_manager.list_accounts() {
            Ok(accounts) => serde_json::to_string(&accounts).unwrap_or_else(|_| "[]".into()),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Save or create a SIP account and store password securely in Secret Service
    async fn save_account(&self, config_json: &str, password: &str) -> String {
        let account: SipAccount = match serde_json::from_str(config_json) {
            Ok(a) => a,
            Err(e) => return format!("Invalid account JSON: {}", e),
        };

        match self.account_manager.save_account(&account, password) {
            Ok(_) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Delete an account and its credentials
    async fn delete_account(&self, account_id: &str) -> String {
        let uuid = match Uuid::parse_str(account_id) {
            Ok(u) => AccountId(u),
            Err(e) => return format!("Invalid UUID: {}", e),
        };

        match self.account_manager.delete_account(uuid).await {
            Ok(_) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Get current registration state for an account
    async fn get_account_status(&self, account_id: &str) -> String {
        let uuid = match Uuid::parse_str(account_id) {
            Ok(u) => AccountId(u),
            Err(e) => return format!("Invalid UUID: {}", e),
        };

        let state = self.account_manager.get_registration_state(uuid).await;
        serde_json::to_string(&state).unwrap_or_else(|_| "\"unregistered\"".into())
    }

    /// Explicitly trigger real SIP registration for an account
    async fn register_account(&self, account_id: &str) -> String {
        let uuid = match Uuid::parse_str(account_id) {
            Ok(u) => AccountId(u),
            Err(e) => return format!("Invalid UUID: {}", e),
        };

        match self.account_manager.register_account(uuid).await {
            Ok(()) => "OK".into(),
            Err(e) => format!("Registration failed: {}", e),
        }
    }

    /// Explicitly unregister a SIP account
    async fn unregister_account(&self, account_id: &str) -> String {
        let uuid = match Uuid::parse_str(account_id) {
            Ok(u) => AccountId(u),
            Err(e) => return format!("Invalid UUID: {}", e),
        };

        match self.account_manager.unregister_account(uuid).await {
            Ok(()) => "OK".into(),
            Err(e) => format!("Unregistration failed: {}", e),
        }
    }

    /// Reconnect and reregister all enabled SIP accounts
    async fn reconnect_all(&self) -> String {
        match self.account_manager.reconnect_all().await {
            Ok(count) => format!("OK: {} accounts reconnected", count),
            Err(e) => format!("Reconnect failed: {}", e),
        }
    }

    /// Start an outgoing call
    async fn start_call(&self, account_id: &str, target: &str) -> String {
        let uuid = if account_id.is_empty() {
            if let Ok(accounts) = self.storage.list_accounts() {
                if let Some(acc) = accounts
                    .iter()
                    .find(|a| a.is_default && a.enabled)
                    .or_else(|| accounts.iter().find(|a| a.enabled))
                {
                    acc.id
                } else {
                    return "Error: No enabled SIP account configured".into();
                }
            } else {
                return "Error: Failed to query accounts".into();
            }
        } else {
            match Uuid::parse_str(account_id) {
                Ok(u) => AccountId(u),
                Err(e) => return format!("Invalid UUID: {}", e),
            }
        };

        match self
            .call_manager
            .start_outgoing_call(uuid, target.to_string())
            .await
        {
            Ok(call_id) => call_id.to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Answer an incoming ringing call
    async fn answer_call(&self, call_id: &str) -> String {
        let uuid = if call_id.is_empty() {
            if let Some(active) = self.call_manager.get_active_call().await {
                active.id
            } else {
                return "Error: No active call to answer".into();
            }
        } else {
            match Uuid::parse_str(call_id) {
                Ok(u) => CallId(u),
                Err(e) => return format!("Invalid UUID: {}", e),
            }
        };

        match self.call_manager.answer_call(uuid).await {
            Ok(_) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Reject an incoming call
    async fn reject_call(&self, call_id: &str) -> String {
        let uuid = if call_id.is_empty() {
            if let Some(active) = self.call_manager.get_active_call().await {
                active.id
            } else {
                return "Error: No active call to reject".into();
            }
        } else {
            match Uuid::parse_str(call_id) {
                Ok(u) => CallId(u),
                Err(e) => return format!("Invalid UUID: {}", e),
            }
        };

        match self.call_manager.reject_call(uuid).await {
            Ok(()) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Hang up an ongoing or dialing call
    async fn hangup_call(&self, call_id: &str) -> String {
        let uuid = if call_id.is_empty() {
            if let Some(active) = self.call_manager.get_active_call().await {
                active.id
            } else {
                return "Error: No active call to hang up".into();
            }
        } else {
            match Uuid::parse_str(call_id) {
                Ok(u) => CallId(u),
                Err(e) => return format!("Invalid UUID: {}", e),
            }
        };

        match self.call_manager.hangup_call(uuid).await {
            Ok(_) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Hold an ongoing call (sends in-dialog re-INVITE with a=sendonly)
    async fn hold_call(&self, call_id: &str) -> String {
        let uuid = if call_id.is_empty() {
            if let Some(active) = self.call_manager.get_active_call().await {
                active.id
            } else {
                return "Error: No active call to hold".into();
            }
        } else {
            match Uuid::parse_str(call_id) {
                Ok(u) => CallId(u),
                Err(e) => return format!("Invalid UUID: {}", e),
            }
        };

        match self.call_manager.hold_call(uuid).await {
            Ok(()) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Resume a held call (sends in-dialog re-INVITE with a=sendrecv)
    async fn resume_call(&self, call_id: &str) -> String {
        let uuid = if call_id.is_empty() {
            if let Some(active) = self.call_manager.get_active_call().await {
                active.id
            } else {
                return "Error: No active call to resume".into();
            }
        } else {
            match Uuid::parse_str(call_id) {
                Ok(u) => CallId(u),
                Err(e) => return format!("Invalid UUID: {}", e),
            }
        };

        match self.call_manager.resume_call(uuid).await {
            Ok(()) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Mute or unmute microphone
    async fn set_muted(&self, muted: bool) -> String {
        self.call_manager.set_muted(muted);
        "OK".into()
    }

    /// Query current mute state
    async fn is_muted(&self) -> bool {
        self.call_manager.is_muted()
    }

    /// Get real-time RTP media statistics for a call
    async fn get_media_stats(&self, call_id: &str) -> String {
        let uuid = match Uuid::parse_str(call_id) {
            Ok(u) => CallId(u),
            Err(e) => return format!("Invalid UUID: {}", e),
        };

        match self.call_manager.get_media_stats(uuid).await {
            Some(stats) => serde_json::to_string(&stats).unwrap_or_else(|_| "{}".into()),
            None => "{}".into(),
        }
    }

    /// Send DTMF telephony event digit
    async fn send_dtmf(&self, call_id: &str, digit: &str) -> String {
        let uuid = match Uuid::parse_str(call_id) {
            Ok(u) => CallId(u),
            Err(e) => return format!("Invalid UUID: {}", e),
        };

        let ch = match digit.chars().next() {
            Some(c) => c,
            None => return "Invalid digit".into(),
        };

        match self.call_manager.send_dtmf(uuid, ch).await {
            Ok(()) => "OK".into(),
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Get currently active call if any
    async fn get_active_call(&self) -> String {
        match self.call_manager.get_active_call().await {
            Some(call) => serde_json::to_string(&call).unwrap_or_else(|_| "null".into()),
            None => "null".into(),
        }
    }

    /// Enumerate audio input and output devices
    async fn get_audio_devices(&self) -> String {
        let inputs = list_input_devices().unwrap_or_default();
        let outputs = list_output_devices().unwrap_or_default();
        serde_json::json!({
            "inputs": inputs,
            "outputs": outputs
        })
        .to_string()
    }

    /// Retrieve audio configuration
    async fn get_audio_config(&self) -> String {
        let cfg = self.storage.get_audio_config().unwrap_or_default();
        serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into())
    }

    /// Save audio configuration
    async fn save_audio_config(&self, config_json: &str) -> String {
        match serde_json::from_str::<AudioConfig>(config_json) {
            Ok(cfg) => {
                if let Err(e) = self.storage.save_audio_config(&cfg) {
                    format!("Error saving audio config: {}", e)
                } else {
                    "OK".into()
                }
            }
            Err(e) => format!("Invalid JSON: {}", e),
        }
    }

    /// Play test ringtone preview
    async fn test_ringtone(&self, ringtone: &str, custom_path: &str, volume: u32) -> String {
        let custom_opt = if custom_path.trim().is_empty() {
            None
        } else {
            Some(custom_path.trim())
        };
        match self
            .call_manager
            .test_ringtone(ringtone, custom_opt, volume)
            .await
        {
            Ok(()) => "OK".into(),
            Err(e) => format!("Error testing ringtone: {}", e),
        }
    }

    /// Stop test ringtone preview
    async fn stop_test_ringtone(&self) -> String {
        self.call_manager.stop_test_ringtone().await;
        "OK".into()
    }

    /// Retrieve configured incoming call reject mode ("603_decline", "486_busy_here", "600_busy_everywhere", "answer_and_drop")
    async fn get_reject_mode(&self) -> String {
        self.storage
            .get_reject_mode()
            .unwrap_or_default()
            .as_str()
            .to_string()
    }

    /// Set incoming call reject mode ("603_decline", "486_busy_here", "600_busy_everywhere", "answer_and_drop")
    async fn set_reject_mode(&self, mode: &str) -> String {
        let parsed = CallRejectMode::from_str_loose(mode);
        match self.storage.save_reject_mode(parsed) {
            Ok(()) => "OK".into(),
            Err(e) => format!("Error saving reject mode: {}", e),
        }
    }

    /// Retrieve recent call history
    async fn get_call_history(&self, limit: u32) -> String {
        match self.storage.list_call_history(limit as usize) {
            Ok(history) => {
                let serialized = history
                    .into_iter()
                    .map(|entry| {
                        let party = sip_protocol::SipPartyInfo::parse(&entry.remote_party);
                        let (display_name, phone_number, display_string) = match party {
                            Some(p) => (p.display_name, p.number_or_user, p.display_string),
                            None => (None, entry.remote_party.clone(), entry.remote_party.clone()),
                        };
                        serde_json::json!({
                            "id": entry.id.to_string(),
                            "account_id": entry.account_id.to_string(),
                            "direction": entry.direction,
                            "remote_party": display_string,
                            "display_name": display_name,
                            "phone_number": phone_number,
                            "started_at": entry.started_at.to_rfc3339(),
                            "start_time": entry.started_at.to_rfc3339(),
                            "duration_seconds": entry.duration_seconds,
                            "duration_secs": entry.duration_seconds,
                            "disconnect_reason": entry.disconnect_reason,
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::to_string(&serialized).unwrap_or_else(|_| "[]".into())
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Clear all call history records
    async fn clear_call_history(&self) -> bool {
        self.storage.clear_call_history().is_ok()
    }

    /// Delete a single call log entry by ID
    async fn delete_call_log_entry(&self, id: &str) -> bool {
        if let Ok(uuid) = uuid::Uuid::parse_str(id) {
            self.storage.delete_call_log(&CallId(uuid)).is_ok()
        } else {
            false
        }
    }

    /// Signal emitted when an account's registration state changes
    #[zbus(signal)]
    pub async fn registration_changed(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        account_id: &str,
        state: &str,
    ) -> zbus::Result<()>;

    /// Signal emitted when an incoming call arrives
    #[zbus(signal)]
    pub async fn incoming_call(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        call_id: &str,
        caller: &str,
    ) -> zbus::Result<()>;

    /// Signal emitted when a call's state changes
    #[zbus(signal)]
    pub async fn call_state_changed(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        call_id: &str,
        state: &str,
    ) -> zbus::Result<()>;

    /// Signal emitted to request opening/raising the graphical user interface window
    #[zbus(signal)]
    pub async fn raise_window(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
}

impl PlasmaSipDbusService {
    /// Spawns an asynchronous background task listening for AccountManager registration events
    /// and publishing them over D-Bus as `registration_changed` signals.
    pub fn spawn_registration_signal_forwarder(
        conn: zbus::Connection,
        object_path: String,
        account_manager: Arc<AccountManager>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx = account_manager.subscribe_registration_events();
            let emitter = match zbus::object_server::SignalEmitter::new(&conn, object_path.as_str())
            {
                Ok(e) => e,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        path = %object_path,
                        "Failed to create SignalEmitter for D-Bus registration signals"
                    );
                    return;
                }
            };

            while let Ok((account_id, state)) = rx.recv().await {
                let state_str = match serde_json::to_string(&state) {
                    Ok(s) => s,
                    Err(_) => format!("{:?}", state),
                };
                if let Err(e) =
                    Self::registration_changed(&emitter, &account_id.to_string(), &state_str).await
                {
                    tracing::warn!(
                        account_id = %account_id,
                        error = %e,
                        "Failed to emit registration_changed D-Bus signal"
                    );
                }
            }
        })
    }

    /// Spawns an asynchronous background task listening for CallManager state events and incoming calls
    /// and publishing them over D-Bus as `call_state_changed` and `incoming_call` signals.
    pub fn spawn_call_signal_forwarder(
        conn: zbus::Connection,
        object_path: String,
        call_manager: Arc<CallManager>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx_state = call_manager.subscribe_call_events();
            let mut rx_incoming = call_manager.subscribe_incoming_calls();
            let emitter = match zbus::object_server::SignalEmitter::new(&conn, object_path.as_str())
            {
                Ok(e) => e,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        path = %object_path,
                        "Failed to create SignalEmitter for D-Bus call state signals"
                    );
                    return;
                }
            };

            loop {
                tokio::select! {
                    Ok((call_id, state)) = rx_state.recv() => {
                        let state_str = match serde_json::to_string(&state) {
                            Ok(s) => s,
                            Err(_) => format!("{:?}", state),
                        };
                        if let Err(e) =
                            Self::call_state_changed(&emitter, &call_id.to_string(), &state_str).await
                        {
                            tracing::warn!(
                                call_id = %call_id,
                                error = %e,
                                "Failed to emit call_state_changed D-Bus signal"
                            );
                        }
                    }
                    Ok((call_id, caller)) = rx_incoming.recv() => {
                        if let Err(e) =
                            Self::incoming_call(&emitter, &call_id.to_string(), &caller).await
                        {
                            tracing::warn!(
                                call_id = %call_id,
                                error = %e,
                                "Failed to emit incoming_call D-Bus signal"
                            );
                        }
                    }
                    else => break,
                }
            }
        })
    }
}
