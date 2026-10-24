//! Call manager orchestrating active calls, incoming ringing, muting, and busy states.

use chrono::Utc;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, info, warn};

use sip_audio::AudioEngine;
use sip_domain::{
    AccountId, ActiveCallInfo, CallDirection, CallId, CallRejectMode, CallState, DisconnectReason,
};
use sip_engine::{CallEvent, IncomingCallSession, IncomingRequestHandler, OutgoingCallSession};
use sip_media::stats::MediaStats;
use sip_platform::PlatformNotifier;
use sip_protocol::{SipMethod, SipRequest, SipResponse, TransactionManager, generate_tag};
use sip_providers::{FritzBoxProvider, GenericSipProvider, SipProviderProfile};
use sip_storage::{CallLogEntry, StorageManager};
use sip_transport::SipUdpTransport;

#[derive(Clone)]
pub enum ActiveSession {
    Outgoing(Arc<OutgoingCallSession>),
    Incoming(Arc<IncomingCallSession>),
}

impl ActiveSession {
    pub fn call_id(&self) -> CallId {
        match self {
            ActiveSession::Outgoing(s) => s.call_id,
            ActiveSession::Incoming(s) => s.call_id,
        }
    }

    pub fn sip_call_id(&self) -> &str {
        match self {
            ActiveSession::Outgoing(s) => &s.sip_call_id,
            ActiveSession::Incoming(s) => &s.sip_call_id,
        }
    }

    pub async fn hold(&self) -> Result<(), String> {
        match self {
            ActiveSession::Outgoing(s) => s.hold().await,
            ActiveSession::Incoming(s) => s.hold().await,
        }
    }

    pub async fn resume(&self) -> Result<(), String> {
        match self {
            ActiveSession::Outgoing(s) => s.resume().await,
            ActiveSession::Incoming(s) => s.resume().await,
        }
    }

    pub async fn handle_reinvite(&self, req: &SipRequest, src: SocketAddr) -> bool {
        match self {
            ActiveSession::Outgoing(s) => s.handle_reinvite(req, src).await,
            ActiveSession::Incoming(s) => s.handle_reinvite(req, src).await,
        }
    }
}

pub struct CallManager {
    storage: Arc<StorageManager>,
    transport: Arc<SipUdpTransport>,
    transactions: Arc<TransactionManager>,
    active_call: Arc<Mutex<Option<ActiveCallInfo>>>,
    active_session: Arc<Mutex<Option<ActiveSession>>>,
    audio_engine: Arc<Mutex<AudioEngine>>,
    notifier: Arc<Mutex<Option<PlatformNotifier>>>,
    is_muted: Arc<AtomicBool>,
    state_tx: broadcast::Sender<(CallId, CallState)>,
    incoming_call_tx: broadcast::Sender<(CallId, String)>,
    test_ringing_stopper: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl CallManager {
    pub fn new(
        storage: Arc<StorageManager>,
        transport: Arc<SipUdpTransport>,
        transactions: Arc<TransactionManager>,
    ) -> Self {
        let (state_tx, _) = broadcast::channel(128);
        let (incoming_call_tx, _) = broadcast::channel(32);
        Self {
            storage,
            transport,
            transactions,
            active_call: Arc::new(Mutex::new(None)),
            active_session: Arc::new(Mutex::new(None)),
            audio_engine: Arc::new(Mutex::new(AudioEngine::new())),
            notifier: Arc::new(Mutex::new(None)),
            is_muted: Arc::new(AtomicBool::new(false)),
            state_tx,
            incoming_call_tx,
            test_ringing_stopper: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_audio_engine(self, engine: AudioEngine) -> Self {
        *self.audio_engine.try_lock().expect("audio engine lock") = engine;
        self
    }

    pub async fn set_audio_engine(&self, engine: AudioEngine) {
        *self.audio_engine.lock().await = engine;
    }

    pub fn with_notifier(self, notifier: PlatformNotifier) -> Self {
        *self.notifier.try_lock().expect("notifier lock") = Some(notifier);
        self
    }

    pub async fn set_notifier(&self, notifier: PlatformNotifier) {
        *self.notifier.lock().await = Some(notifier);
    }

    /// Subscribes to call state events (e.g. for D-Bus signals or UI updates)
    pub fn subscribe_call_events(&self) -> broadcast::Receiver<(CallId, CallState)> {
        self.state_tx.subscribe()
    }

    /// Subscribes to incoming call events (call_id, caller)
    pub fn subscribe_incoming_calls(&self) -> broadcast::Receiver<(CallId, String)> {
        self.incoming_call_tx.subscribe()
    }

    pub async fn get_active_call(&self) -> Option<ActiveCallInfo> {
        self.active_call.lock().await.clone()
    }

    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::SeqCst);
        let audio_engine = Arc::clone(&self.audio_engine);
        tokio::spawn(async move {
            audio_engine.lock().await.set_muted(muted);
        });
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::SeqCst)
    }

    pub async fn get_media_stats(&self, call_id: CallId) -> Option<MediaStats> {
        let session_guard = self.active_session.lock().await;
        if let Some(ref sess) = *session_guard {
            match sess {
                ActiveSession::Outgoing(s) if s.call_id == call_id => {
                    return s.get_media_stats().await;
                }
                ActiveSession::Incoming(s) if s.call_id == call_id => {
                    return s.get_media_stats().await;
                }
                _ => {}
            }
        }
        None
    }

    pub async fn send_dtmf(&self, call_id: CallId, digit: char) -> Result<(), String> {
        let session_guard = self.active_session.lock().await;
        if let Some(ref sess) = *session_guard {
            match sess {
                ActiveSession::Outgoing(s) if s.call_id == call_id => {
                    return s.send_dtmf(digit).await;
                }
                ActiveSession::Incoming(s) if s.call_id == call_id => {
                    return s.send_dtmf(digit).await;
                }
                _ => {}
            }
        }
        Err("No matching active call session found".into())
    }

    /// Initiates a genuine outgoing SIP call over the network with full RFC 3261 signaling.
    pub async fn start_outgoing_call(
        &self,
        account_id: AccountId,
        remote_party: String,
    ) -> Result<CallId, String> {
        // 1. Verify no existing active call (line busy)
        {
            let active = self.active_call.lock().await;
            if active.is_some() {
                return Err("Another call is already in progress (line busy)".into());
            }
        }

        // 2. Load account configuration from SQLite
        let account = self
            .storage
            .get_account(account_id)
            .map_err(|e| format!("Storage error: {}", e))?
            .ok_or_else(|| format!("Account {} not found", account_id))?;

        if !account.enabled {
            return Err("Cannot place call from a disabled account".into());
        }

        // 3. Load credentials from secure storage
        let password = match self.storage.get_account_secret(account_id) {
            Ok(Some(pw)) => pw,
            Ok(None) => {
                return Err(format!(
                    "No credentials found in Secret Service for account {}",
                    account_id
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Failed to retrieve credentials from Secret Service: {}",
                    e
                ));
            }
        };

        // 4. Normalize dial target according to provider profile
        let dial_target = if account.domain == "fritz.box" || account.registrar == "fritz.box" {
            FritzBoxProvider.normalize_dial_target(&remote_party, &account)
        } else {
            GenericSipProvider.normalize_dial_target(&remote_party, &account)
        }
        .map_err(|e| format!("Dial target normalization failed: {}", e))?;

        // 5. RFC 3263 DNS resolution of registrar / SIP proxy
        let dest_addr =
            match sip_transport::resolve_sip_target(&account.registrar, account.registrar_port)
                .await
            {
                Ok(addrs) => match self.transport.select_compatible_addr(addrs) {
                    Some(addr) => addr,
                    None => {
                        return Err(format!(
                            "DNS lookup for {}:{} returned no compatible addresses",
                            account.registrar, account.registrar_port
                        ));
                    }
                },
                Err(e) => {
                    return Err(format!(
                        "DNS resolution failed for {}:{}: {}",
                        account.registrar, account.registrar_port, e
                    ));
                }
            };

        // 6. Create call tracking record
        self.stop_test_ringtone().await;
        let call_id = CallId::new();
        let call_info = ActiveCallInfo {
            id: call_id,
            account_id,
            direction: CallDirection::Outbound,
            remote_party: remote_party.clone(),
            state: CallState::Dialing,
            muted: false,
            created_at: Utc::now(),
            connected_at: None,
        };

        *self.active_call.lock().await = Some(call_info);
        let _ = self.state_tx.send((call_id, CallState::Dialing));

        // 7. Instantiate OutgoingCallSession in sip-engine
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);
        let rtp_port = std::net::UdpSocket::bind("127.0.0.1:0")
            .ok()
            .and_then(|s| s.local_addr().ok())
            .map(|a| a.port())
            .unwrap_or(16000);
        let session = Arc::new(OutgoingCallSession::with_call_id(
            call_id,
            account.clone(),
            password,
            dial_target.normalized_sip_uri.clone(),
            rtp_port,
            self.transport.clone(),
            self.transactions.clone(),
            event_tx,
        ));

        *self.active_session.lock().await = Some(ActiveSession::Outgoing(session.clone()));

        // 8. Spawn background event forwarder and lifecycle watcher
        let active_call_clone = self.active_call.clone();
        let active_session_clone = self.active_session.clone();
        let state_tx_clone = self.state_tx.clone();
        let storage_clone = self.storage.clone();
        let audio_engine_clone = self.audio_engine.clone();
        let notifier_clone = self.notifier.clone();
        let session_for_audio = session.clone();

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    CallEvent::StateChanged(new_state) => {
                        let mut call_ended = false;
                        let mut entry_to_log = None;
                        let mut broadcast_state = new_state.clone();

                        {
                            let mut active = active_call_clone.lock().await;
                            if let Some(ref mut call) = *active
                                && call.id == call_id
                            {
                                let effective_state = match new_state {
                                    CallState::Active { started_at } => {
                                        let actual_start = call.connected_at.unwrap_or(started_at);
                                        CallState::Active {
                                            started_at: actual_start,
                                        }
                                    }
                                    s => s,
                                };
                                call.state = effective_state.clone();
                                broadcast_state = effective_state;

                                if let CallState::Active { started_at } = call.state {
                                    if call.connected_at.is_none() {
                                        call.connected_at = Some(started_at);

                                        // Connect real-time audio pipeline
                                        let audio_cfg =
                                            storage_clone.get_audio_config().unwrap_or_default();
                                        let in_dev = audio_cfg.input_device_id.as_deref();
                                        let out_dev = audio_cfg.output_device_id.as_deref();

                                        let (pcm_rx, _dtmf_rx) =
                                            session_for_audio.take_audio_channels().await;
                                        let mut engine = audio_engine_clone.lock().await;
                                        if let Some(rx) = pcm_rx {
                                            let _ = engine.start_output(out_dev, rx);
                                        }
                                        let (mic_tx, mut mic_rx) = tokio::sync::mpsc::channel(32);
                                        if engine.start_input(in_dev, mic_tx).is_ok() {
                                            let sess = session_for_audio.clone();
                                            tokio::spawn(async move {
                                                while let Some(frame) = mic_rx.recv().await {
                                                    if sess.send_audio_frame(frame).await.is_err() {
                                                        break;
                                                    }
                                                }
                                            });
                                        }
                                    }
                                } else if let CallState::Terminated { ref reason } = call.state {
                                    call_ended = true;
                                    let duration_secs = call
                                        .connected_at
                                        .map(|c| (Utc::now() - c).num_seconds().max(0) as u32)
                                        .unwrap_or(0);

                                    // Stop audio streams on call termination
                                    audio_engine_clone.lock().await.stop();

                                    entry_to_log = Some(CallLogEntry {
                                        id: call.id,
                                        account_id: call.account_id,
                                        direction: call.direction,
                                        remote_party: call.remote_party.clone(),
                                        started_at: call.created_at,
                                        duration_seconds: duration_secs,
                                        disconnect_reason: reason.to_string(),
                                    });
                                }
                            }

                            if call_ended {
                                *active = None;
                            }
                        }

                        let _ = state_tx_clone.send((call_id, broadcast_state));

                        if let Some(entry) = entry_to_log {
                            let _ = storage_clone.log_call(&entry);
                            *active_session_clone.lock().await = None;

                            let notif_guard = notifier_clone.lock().await;
                            if let Some(ref notif) = *notif_guard {
                                let _ = notif.close_call_notification(call_id).await;
                            }
                        }
                    }

                    CallEvent::RemoteSdpNegotiated(_sdp) => {
                        info!(
                            call_id = %call_id,
                            "Remote SDP negotiated for outgoing call"
                        );
                    }
                    CallEvent::EarlyMediaStarted => {
                        info!(call_id = %call_id, "Connecting speaker output for early media ringback");
                        let audio_cfg = storage_clone.get_audio_config().unwrap_or_default();
                        let out_dev = audio_cfg.output_device_id.as_deref();
                        let (pcm_rx, _dtmf_rx) = session_for_audio.take_audio_channels().await;
                        let mut engine = audio_engine_clone.lock().await;
                        if let Some(rx) = pcm_rx {
                            let _ = engine.start_output(out_dev, rx);
                        }
                    }
                    CallEvent::Error(err) => {
                        warn!(call_id = %call_id, error = %err, "Call event error");
                    }
                }
            }
        });

        // 9. Spawn the network session runner
        let session_runner = session.clone();
        let state_tx_fail = self.state_tx.clone();
        let active_call_fail = self.active_call.clone();
        let active_session_fail = self.active_session.clone();
        let storage_fail = self.storage.clone();
        let notifier_fail = self.notifier.clone();
        tokio::spawn(async move {
            if let Err(e) = session_runner.start(dest_addr).await {
                info!(call_id = %call_id, reason = %e, "Outgoing call handshake failed");
                // Handshake failed: Ensure call state is cleaned up so it does not get stuck in RingingRemote or Dialing
                let mut entry_to_log = None;
                {
                    let mut active = active_call_fail.lock().await;
                    if let Some(ref call) = *active
                        && call.id == call_id
                        && !matches!(call.state, CallState::Terminated { .. })
                    {
                        entry_to_log = Some(CallLogEntry {
                            id: call.id,
                            account_id: call.account_id,
                            direction: call.direction,
                            remote_party: call.remote_party.clone(),
                            started_at: call.created_at,
                            duration_seconds: 0,
                            disconnect_reason: format!("Failed or timed out: {}", e),
                        });
                        *active = None;
                    }
                }
                if let Some(entry) = entry_to_log {
                    *active_session_fail.lock().await = None;
                    let _ = state_tx_fail.send((
                        call_id,
                        CallState::Terminated {
                            reason: DisconnectReason::Timeout,
                        },
                    ));
                    let _ = storage_fail.log_call(&entry);
                    let notif = notifier_fail.lock().await;
                    if let Some(ref n) = *notif {
                        let _ = n.close_call_notification(call_id).await;
                    }
                }
            }
        });

        info!(
            call_id = %call_id,
            is_internal = dial_target.is_internal_extension,
            "Outgoing call initiated over SIP"
        );
        Ok(call_id)
    }

    /// Hang up an ongoing or dialing call (issuing CANCEL or BYE as appropriate)
    pub async fn hangup_call(&self, call_id: CallId) -> Result<(), String> {
        let session = self.active_session.lock().await.clone();
        let current_state = {
            let active = self.active_call.lock().await;
            match *active {
                Some(ref c) if c.id == call_id => Some(c.state.clone()),
                _ => None,
            }
        };

        if let Some(state) = current_state {
            self.audio_engine.lock().await.stop();
            let disconnect_reason = match state {
                CallState::Dialing | CallState::RingingRemote => DisconnectReason::Cancelled,
                _ => DisconnectReason::LocalHangup,
            };

            if let Some(sess) = session {
                match sess {
                    ActiveSession::Outgoing(s) if s.call_id == call_id => match state {
                        CallState::Dialing | CallState::RingingRemote => {
                            let _ = s.cancel_current().await;
                        }
                        _ => {
                            let _ = s.hangup_current().await;
                        }
                    },
                    ActiveSession::Incoming(s) if s.call_id == call_id => {
                        match s.get_state().await {
                            CallState::RingingLocal => {
                                let _ = s.reject(603, "Decline").await;
                            }
                            _ => {
                                let _ = s.hangup().await;
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Unconditionally terminate and clean up active call state locally
            self.end_call(call_id, disconnect_reason).await;

            let notif = self.notifier.lock().await;
            if let Some(ref n) = *notif {
                let _ = n.close_call_notification(call_id).await;
            }

            Ok(())
        } else {
            Err(format!("Call {} not found", call_id))
        }
    }

    /// Places an ongoing call on hold (sends in-dialog re-INVITE with a=sendonly)
    pub async fn hold_call(&self, call_id: CallId) -> Result<(), String> {
        let session = {
            let guard = self.active_session.lock().await;
            match guard.as_ref() {
                Some(s) if s.call_id() == call_id => s.clone(),
                _ => return Err(format!("Call {} not found or inactive", call_id)),
            }
        };

        session.hold().await?;

        let mut call_guard = self.active_call.lock().await;
        if let Some(ref mut c) = *call_guard
            && c.id == call_id
        {
            c.state = CallState::Held;
        }
        let _ = self.state_tx.send((call_id, CallState::Held));
        info!(call_id = %call_id, "Call transitioned to Held state");
        Ok(())
    }

    /// Resumes a held call (sends in-dialog re-INVITE with a=sendrecv)
    pub async fn resume_call(&self, call_id: CallId) -> Result<(), String> {
        let session = {
            let guard = self.active_session.lock().await;
            match guard.as_ref() {
                Some(s) if s.call_id() == call_id => s.clone(),
                _ => return Err(format!("Call {} not found or inactive", call_id)),
            }
        };

        session.resume().await?;

        let started_at = {
            let mut call_guard = self.active_call.lock().await;
            if let Some(ref mut c) = *call_guard {
                if c.id == call_id {
                    let start = c.connected_at.unwrap_or_else(Utc::now);
                    c.state = CallState::Active { started_at: start };
                    start
                } else {
                    Utc::now()
                }
            } else {
                Utc::now()
            }
        };
        let _ = self
            .state_tx
            .send((call_id, CallState::Active { started_at }));
        info!(call_id = %call_id, "Call resumed to Active state");
        Ok(())
    }

    /// Handles an incoming in-dialog BYE request from the remote endpoint
    pub async fn handle_incoming_bye(&self, req: SipRequest, src: SocketAddr) -> bool {
        let session = self.active_session.lock().await.clone();
        if let Some(sess) = session {
            match sess {
                ActiveSession::Outgoing(ref s) => {
                    if s.handle_incoming_bye(&req, src).await.is_some() {
                        self.audio_engine.lock().await.stop();
                        return true;
                    }
                }
                ActiveSession::Incoming(ref s) => {
                    if let Some(call_id_hdr) = req.call_id()
                        && s.sip_call_id == call_id_hdr
                    {
                        let _ = s.handle_bye(&req).await;
                        self.audio_engine.lock().await.stop();
                        return true;
                    }
                }
            }
        }

        // Return 481 Call/Transaction Does Not Exist if no matching dialog
        let resp = SipResponse::create_for_request(
            &req,
            481,
            "Call/Transaction Does Not Exist",
            None,
            None,
            None,
        );

        let _ = self
            .transport
            .send_to(resp.serialize().as_bytes(), src)
            .await;
        false
    }

    /// Handles an incoming INVITE request from a remote endpoint
    pub async fn handle_incoming_invite(&self, req: SipRequest, src: SocketAddr) -> bool {
        // 1. Check if this is an in-dialog re-INVITE for the current active call
        {
            let session_opt = self.active_session.lock().await.clone();
            if let Some(session) = session_opt
                && session.handle_reinvite(&req, src).await
            {
                info!("Handled in-dialog re-INVITE successfully");
                return true;
            }
        }

        // 2. Check busy state
        {
            let active = self.active_call.lock().await;
            if active.is_some() {
                info!("Received incoming INVITE while line is busy; responding 486 Busy Here");
                let to_tag = generate_tag();
                let busy_res = SipResponse::create_for_request(
                    &req,
                    486,
                    "Busy Here",
                    Some(&to_tag),
                    None,
                    None,
                );
                let _ = self
                    .transport
                    .send_to(busy_res.serialize().as_bytes(), src)
                    .await;
                return true;
            }
        }

        // 2. Validate source IP origin against registered PBX / registrar (SEC-03 Phantom Call Protection)
        let accounts = self.storage.list_accounts().unwrap_or_default();
        let is_loopback = src.ip().is_loopback();
        let mut is_trusted_origin = is_loopback;

        if !is_trusted_origin {
            for a in accounts.iter().filter(|a| a.enabled) {
                if a.registrar == src.ip().to_string() {
                    is_trusted_origin = true;
                    break;
                }
                if let Ok(addrs) =
                    sip_transport::resolve_sip_target(&a.registrar, a.registrar_port).await
                    && addrs.into_iter().any(|addr| addr.ip() == src.ip())
                {
                    is_trusted_origin = true;
                    break;
                }
            }
        }

        if !is_trusted_origin {
            warn!(
                "Rejecting incoming INVITE from untrusted origin (not matching any enabled account registrar)"
            );
            let to_tag = generate_tag();
            let reject_res =
                SipResponse::create_for_request(&req, 403, "Forbidden", Some(&to_tag), None, None);
            let _ = self
                .transport
                .send_to(reject_res.serialize().as_bytes(), src)
                .await;
            return true;
        }

        // 3. Match account from SQLite
        let matched_account = accounts
            .iter()
            .find(|a| {
                if let Some(to) = req.headers.get("To") {
                    to.contains(&a.username) || to.contains(&a.registrar)
                } else {
                    false
                }
            })
            .or_else(|| accounts.iter().find(|a| a.enabled))
            .cloned();

        let account_id = matched_account
            .as_ref()
            .map(|a| a.id)
            .unwrap_or_else(AccountId::new);

        // 3. Extract caller identity and called party (line)
        let caller_party = req.caller_info();
        let caller = caller_party.display_string.clone();
        let called_party = req.called_info();
        let called_line = called_party.as_ref().map(|c| c.number_or_user.clone());

        let has_caller_display = caller_party.display_name.is_some();
        let has_called_line = called_line.is_some();
        debug!(
            has_display_name = has_caller_display,
            has_called_line = has_called_line,
            "Processing incoming SIP INVITE"
        );

        // 4. Create and initialize incoming session (sends 100 Trying and 180 Ringing)
        let call_id = CallId::new();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);

        let session = match IncomingCallSession::with_call_id_and_ring(
            call_id,
            req.clone(),
            src,
            self.transport.clone(),
            event_tx,
        )
        .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!(error = %e, "Failed to initialize incoming call session");
                return false;
            }
        };

        let call_info = ActiveCallInfo {
            id: call_id,
            account_id,
            direction: CallDirection::Inbound,
            remote_party: caller.clone(),
            state: CallState::RingingLocal,
            muted: false,
            created_at: Utc::now(),
            connected_at: None,
        };

        *self.active_call.lock().await = Some(call_info);
        *self.active_session.lock().await = Some(ActiveSession::Incoming(session.clone()));

        let _ = self.state_tx.send((call_id, CallState::RingingLocal));
        let _ = self.incoming_call_tx.send((call_id, caller.clone()));

        // Trigger desktop notification with clean caller number and called line
        let notifier_guard = self.notifier.lock().await;
        if let Some(ref notif) = *notifier_guard {
            let _ = notif
                .notify_incoming_call(
                    call_id,
                    &caller_party.number_or_user,
                    caller_party.display_name.as_deref(),
                    called_line.as_deref(),
                )
                .await;
        }
        drop(notifier_guard);

        self.stop_test_ringtone().await;

        // Audible Ringtone: Play configured ringtone (custom or synthesized)
        let stop_ringing = Arc::new(AtomicBool::new(false));
        let audio_cfg = self.storage.get_audio_config().unwrap_or_default();

        if !audio_cfg.silent_mode {
            let (ring_tx, ring_rx) = tokio::sync::mpsc::channel(32);
            let ring_dev = audio_cfg.ringing_device_id.clone();

            info!(
                ringing_device = ?ring_dev,
                ringtone = %audio_cfg.ringtone,
                volume = audio_cfg.ringer_volume,
                "Starting audible ringtone for incoming call"
            );

            {
                let mut engine = self.audio_engine.lock().await;
                if let Err(e) = engine.start_output(ring_dev.as_deref(), ring_rx) {
                    warn!(error = %e, "Failed to start ringtone audio output");
                }
            }

            let stop_ringing_task = stop_ringing.clone();
            let ringtone_frames = sip_audio::generate_ringtone_frames(
                &audio_cfg.ringtone,
                audio_cfg.custom_ringtone_path.as_deref(),
                audio_cfg.ringer_volume,
            );

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(20));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                let total_frames = ringtone_frames.len();
                let mut idx = 0usize;

                while !stop_ringing_task.load(Ordering::Relaxed) {
                    interval.tick().await;
                    if stop_ringing_task.load(Ordering::Relaxed) {
                        break;
                    }
                    let frame = ringtone_frames[idx].clone();
                    idx = (idx + 1) % total_frames;
                    if ring_tx.send(frame).await.is_err() {
                        break;
                    }
                }
            });
        } else {
            info!("Silent mode is active: suppressing audible ringtone");
        }

        // 5. Spawn background event forwarder & lifecycle watcher
        let active_call_clone = self.active_call.clone();
        let active_session_clone = self.active_session.clone();
        let state_tx_clone = self.state_tx.clone();
        let storage_clone = self.storage.clone();
        let audio_engine_clone = self.audio_engine.clone();
        let notifier_clone = self.notifier.clone();
        let session_for_audio = session.clone();
        let stop_ringing_watcher = stop_ringing.clone();

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let CallEvent::StateChanged(new_state) = event {
                    if !matches!(new_state, CallState::RingingLocal) {
                        stop_ringing_watcher.store(true, Ordering::Relaxed);
                    }
                    let mut call_ended = false;
                    let mut entry_to_log = None;
                    let mut broadcast_state = new_state.clone();

                    {
                        let mut active = active_call_clone.lock().await;
                        if let Some(ref mut call) = *active
                            && call.id == call_id
                        {
                            let effective_state = match new_state {
                                CallState::Active { started_at } => {
                                    let actual_start = call.connected_at.unwrap_or(started_at);
                                    CallState::Active {
                                        started_at: actual_start,
                                    }
                                }
                                s => s,
                            };
                            call.state = effective_state.clone();
                            broadcast_state = effective_state;

                            if let CallState::Active { started_at } = call.state {
                                if call.connected_at.is_none() {
                                    call.connected_at = Some(started_at);

                                    // Connect real-time audio pipeline
                                    let audio_cfg =
                                        storage_clone.get_audio_config().unwrap_or_default();
                                    let in_dev = audio_cfg.input_device_id.as_deref();
                                    let out_dev = audio_cfg.output_device_id.as_deref();

                                    let (pcm_rx, _dtmf_rx) =
                                        session_for_audio.take_audio_channels().await;
                                    let mut engine = audio_engine_clone.lock().await;
                                    if let Some(rx) = pcm_rx {
                                        let _ = engine.start_output(out_dev, rx);
                                    }
                                    let (mic_tx, mut mic_rx) = tokio::sync::mpsc::channel(32);
                                    if engine.start_input(in_dev, mic_tx).is_ok() {
                                        let sess = session_for_audio.clone();
                                        tokio::spawn(async move {
                                            while let Some(frame) = mic_rx.recv().await {
                                                if sess.send_audio_frame(frame).await.is_err() {
                                                    break;
                                                }
                                            }
                                        });
                                    }
                                }
                            } else if let CallState::Terminated { ref reason } = call.state {
                                call_ended = true;
                                let duration_secs = call
                                    .connected_at
                                    .map(|c| (Utc::now() - c).num_seconds().max(0) as u32)
                                    .unwrap_or(0);

                                audio_engine_clone.lock().await.stop();

                                entry_to_log = Some(CallLogEntry {
                                    id: call.id,
                                    account_id: call.account_id,
                                    direction: call.direction,
                                    remote_party: call.remote_party.clone(),
                                    started_at: call.created_at,
                                    duration_seconds: duration_secs,
                                    disconnect_reason: reason.to_string(),
                                });
                            }
                        }

                        if call_ended {
                            *active = None;
                        }
                    }

                    let _ = state_tx_clone.send((call_id, broadcast_state));

                    if let Some(entry) = entry_to_log {
                        let _ = storage_clone.log_call(&entry);
                        *active_session_clone.lock().await = None;

                        let notif_guard = notifier_clone.lock().await;
                        if let Some(ref notif) = *notif_guard {
                            let _ = notif.close_call_notification(call_id).await;
                        }
                    }
                }
            }
        });

        true
    }

    /// Handles an incoming CANCEL request from remote party before answer
    pub async fn handle_incoming_cancel(&self, req: SipRequest, _src: SocketAddr) -> bool {
        let call_id_str = match req.call_id() {
            Some(id) => id,
            None => return false,
        };

        let session = self.active_session.lock().await.clone();
        if let Some(ActiveSession::Incoming(ref s)) = session
            && s.sip_call_id == call_id_str
        {
            if let Err(e) = s.handle_cancel(&req).await {
                warn!(error = %e, "Failed to handle incoming CANCEL");
            }
            return true;
        }
        false
    }

    /// Handles an incoming ACK request for an answered incoming/outgoing call or re-INVITE
    pub async fn handle_incoming_ack(&self, req: SipRequest, _src: SocketAddr) -> bool {
        let call_id_str = match req.call_id() {
            Some(id) => id,
            None => return false,
        };

        let session = self.active_session.lock().await.clone();
        if let Some(ref sess) = session
            && sess.sip_call_id() == call_id_str
        {
            if !req.body.is_empty()
                && let Ok(sdp) = sip_protocol::SdpSession::parse(&req.body)
            {
                let new_dest = SocketAddr::new(sdp.connection_ip, sdp.audio_port);
                match sess {
                    ActiveSession::Outgoing(s) => {
                        if let Some(ref m) = *s.media_session.lock().await {
                            m.update_destination(new_dest).await;
                        }
                    }
                    ActiveSession::Incoming(s) => {
                        if let Some(ref m) = *s.media_session.lock().await {
                            m.update_destination(new_dest).await;
                        }
                    }
                }
            }
            debug!(call_id = %sess.call_id(), "Absorbed incoming in-dialog ACK");
            return true;
        }
        false
    }

    pub async fn register_incoming_call(
        &self,
        account_id: AccountId,
        remote_party: String,
    ) -> Result<CallId, String> {
        let mut active = self.active_call.lock().await;
        if active.is_some() {
            return Err("Line is busy".into());
        }

        let call_id = CallId::new();
        let call_info = ActiveCallInfo {
            id: call_id,
            account_id,
            direction: CallDirection::Inbound,
            remote_party,
            state: CallState::RingingLocal,
            muted: false,
            created_at: Utc::now(),
            connected_at: None,
        };

        info!(call_id = %call_id, "Registered incoming call");
        *active = Some(call_info);
        let _ = self.state_tx.send((call_id, CallState::RingingLocal));
        Ok(call_id)
    }

    pub async fn answer_call(&self, call_id: CallId) -> Result<(), String> {
        let session = self.active_session.lock().await.clone();
        if let Some(ActiveSession::Incoming(ref s)) = session
            && s.call_id == call_id
        {
            s.answer(0, sip_domain::AudioCodec::Pcma).await?;

            let notif = self.notifier.lock().await;
            if let Some(ref n) = *notif {
                let _ = n.close_call_notification(call_id).await;
            }
            return Ok(());
        }

        let mut active = self.active_call.lock().await;
        if let Some(call) = active.as_mut()
            && call.id == call_id
        {
            let now = Utc::now();
            call.state = CallState::Active { started_at: now };
            call.connected_at = Some(now);
            info!(call_id = %call_id, "Answered incoming call");
            let _ = self
                .state_tx
                .send((call_id, CallState::Active { started_at: now }));
            return Ok(());
        }
        Err(format!(
            "Incoming call {} not found or cannot be answered",
            call_id
        ))
    }

    pub async fn reject_call(&self, call_id: CallId) -> Result<(), String> {
        let reject_mode = self.storage.get_reject_mode().unwrap_or_default();
        let session = self.active_session.lock().await.clone();
        if let Some(ActiveSession::Incoming(ref s)) = session
            && s.call_id == call_id
        {
            self.audio_engine.lock().await.stop();
            match reject_mode {
                CallRejectMode::Decline603 => {
                    let _ = s.reject(603, "Decline").await;
                }
                CallRejectMode::BusyHere486 => {
                    let _ = s.reject(486, "Busy Here").await;
                }
                CallRejectMode::BusyEverywhere600 => {
                    let _ = s.reject(600, "Busy Everywhere").await;
                }
                CallRejectMode::AnswerAndDrop => {
                    let _ = s.answer_and_drop().await;
                }
            }

            let notif = self.notifier.lock().await;
            if let Some(ref n) = *notif {
                let _ = n.close_call_notification(call_id).await;
            }
            return Ok(());
        }
        self.end_call(call_id, DisconnectReason::Rejected).await;
        Ok(())
    }

    pub async fn update_call_state(&self, call_id: CallId, state: CallState) {
        let mut active = self.active_call.lock().await;
        if let Some(call) = active.as_mut()
            && call.id == call_id
        {
            call.state = state.clone();
            let _ = self.state_tx.send((call_id, state));
        }
    }

    pub async fn end_call(&self, call_id: CallId, reason: DisconnectReason) {
        let mut entry_to_log = None;
        {
            let mut active = self.active_call.lock().await;
            if let Some(ref call) = *active
                && call.id == call_id
            {
                let duration_secs = call
                    .connected_at
                    .map(|c| (Utc::now() - c).num_seconds().max(0) as u32)
                    .unwrap_or(0);

                entry_to_log = Some(CallLogEntry {
                    id: call.id,
                    account_id: call.account_id,
                    direction: call.direction,
                    remote_party: call.remote_party.clone(),
                    started_at: call.created_at,
                    duration_seconds: duration_secs,
                    disconnect_reason: reason.to_string(),
                });
                *active = None;
            }
        }

        *self.active_session.lock().await = None;
        let _ = self
            .state_tx
            .send((call_id, CallState::Terminated { reason }));

        if let Some(entry) = entry_to_log {
            let _ = self.storage.log_call(&entry);
            let notif = self.notifier.lock().await;
            if let Some(ref n) = *notif {
                let _ = n.close_call_notification(call_id).await;
            }
        }
    }

    /// Plays a test ringtone preview through the configured ringing output device for up to 6 seconds.
    pub async fn test_ringtone(
        &self,
        ringtone: &str,
        custom_path: Option<&str>,
        volume: u32,
    ) -> Result<(), String> {
        if self.active_call.lock().await.is_some() {
            return Err("Cannot test ringtone while a call is active".to_string());
        }

        self.stop_test_ringtone().await;

        let audio_cfg = self.storage.get_audio_config().unwrap_or_default();
        let ring_dev = audio_cfg.ringing_device_id.clone();
        let frames = sip_audio::generate_ringtone_frames(ringtone, custom_path, volume);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let (ring_tx, ring_rx) = tokio::sync::mpsc::channel(32);

        {
            let mut engine = self.audio_engine.lock().await;
            if let Err(e) = engine.start_output(ring_dev.as_deref(), ring_rx) {
                return Err(format!("Failed to start audio output: {}", e));
            }
        }

        let stop_task = stop_flag.clone();
        let engine_clone = self.audio_engine.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(20));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            let total_frames = frames.len();
            let mut idx = 0usize;
            // Play for at most 6 seconds (300 ticks of 20ms)
            for _ in 0..300 {
                interval.tick().await;
                if stop_task.load(Ordering::Relaxed) {
                    break;
                }
                let frame = frames[idx].clone();
                idx = (idx + 1) % total_frames;
                if ring_tx.send(frame).await.is_err() {
                    break;
                }
            }

            if !stop_task.load(Ordering::Relaxed) {
                let mut engine = engine_clone.lock().await;
                engine.stop_output();
            }
        });

        *self.test_ringing_stopper.lock().await = Some(stop_flag);
        Ok(())
    }

    /// Stops any running test ringtone preview.
    pub async fn stop_test_ringtone(&self) {
        let mut stopper = self.test_ringing_stopper.lock().await;
        if let Some(flag) = stopper.take() {
            flag.store(true, Ordering::Relaxed);
            let mut engine = self.audio_engine.lock().await;
            engine.stop_output();
        }
    }
}

#[async_trait::async_trait]
impl IncomingRequestHandler for CallManager {
    async fn handle_request(&self, req: SipRequest, src: SocketAddr) -> bool {
        match req.method {
            SipMethod::Invite => self.handle_incoming_invite(req, src).await,
            SipMethod::Cancel => self.handle_incoming_cancel(req, src).await,
            SipMethod::Ack => self.handle_incoming_ack(req, src).await,
            SipMethod::Bye => self.handle_incoming_bye(req, src).await,
            _ => false,
        }
    }
}
