//! Outgoing SIP Call Session managing INVITE, SDP offer, provisional responses, ACK, CANCEL, and BYE.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use sip_auth::{DigestAuthorizer, DigestChallenge};
use sip_domain::{AudioCodec, CallId, CallState, DisconnectReason, SipAccount};
use sip_media::{MediaSession, MediaSessionConfig, dtmf::DtmfEvent, stats::MediaStats};
use sip_protocol::{
    SdpSession, SipDialog, SipMethod, SipRequest, SipResponse, TransactionKey, TransactionManager,
    generate_branch, generate_tag,
};
use sip_transport::SipUdpTransport;
use zeroize::Zeroizing;

#[derive(Debug, Clone)]
pub enum CallEvent {
    StateChanged(CallState),
    RemoteSdpNegotiated(SdpSession),
    EarlyMediaStarted,
    Error(String),
}

pub struct OutgoingCallSession {
    pub call_id: CallId,
    pub sip_call_id: String,
    pub account: SipAccount,
    pub password: Zeroizing<String>,
    pub target_uri: String,
    pub local_rtp_port: u16,
    pub transport: Arc<SipUdpTransport>,
    pub transactions: Arc<TransactionManager>,
    pub state: Arc<Mutex<CallState>>,
    pub dialog: Arc<Mutex<Option<SipDialog>>>,
    pub active_invite: Arc<Mutex<Option<SipRequest>>>,
    pub dest_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub media_session: Arc<Mutex<Option<MediaSession>>>,
    pub pcm_out_rx: Arc<Mutex<Option<mpsc::Receiver<Vec<i16>>>>>,
    pub dtmf_rx: Arc<Mutex<Option<mpsc::Receiver<DtmfEvent>>>>,
    current_cseq: Arc<Mutex<u32>>,
    last_invite_branch: Arc<Mutex<Option<String>>>,
    event_tx: mpsc::Sender<CallEvent>,
}

impl OutgoingCallSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: SipAccount,
        password: String,
        target_uri: String,
        local_rtp_port: u16,
        transport: Arc<SipUdpTransport>,
        transactions: Arc<TransactionManager>,
        event_tx: mpsc::Sender<CallEvent>,
    ) -> Self {
        Self::with_call_id(
            CallId::new(),
            account,
            password,
            target_uri,
            local_rtp_port,
            transport,
            transactions,
            event_tx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_call_id(
        call_id: CallId,
        account: SipAccount,
        password: String,
        target_uri: String,
        local_rtp_port: u16,
        transport: Arc<SipUdpTransport>,
        transactions: Arc<TransactionManager>,
        event_tx: mpsc::Sender<CallEvent>,
    ) -> Self {
        let sip_call_id = format!("call-{}@plasma-sip", call_id);
        Self {
            call_id,
            sip_call_id,
            account,
            password: Zeroizing::new(password),
            target_uri,
            local_rtp_port,
            transport,
            transactions,
            state: Arc::new(Mutex::new(CallState::Dialing)),
            dialog: Arc::new(Mutex::new(None)),
            active_invite: Arc::new(Mutex::new(None)),
            dest_addr: Arc::new(Mutex::new(None)),
            media_session: Arc::new(Mutex::new(None)),
            pcm_out_rx: Arc::new(Mutex::new(None)),
            dtmf_rx: Arc::new(Mutex::new(None)),
            current_cseq: Arc::new(Mutex::new(1)),
            last_invite_branch: Arc::new(Mutex::new(None)),
            event_tx,
        }
    }

    pub async fn get_state(&self) -> CallState {
        self.state.lock().await.clone()
    }

    pub async fn take_audio_channels(
        &self,
    ) -> (
        Option<mpsc::Receiver<Vec<i16>>>,
        Option<mpsc::Receiver<DtmfEvent>>,
    ) {
        let pcm = self.pcm_out_rx.lock().await.take();
        let dtmf = self.dtmf_rx.lock().await.take();
        (pcm, dtmf)
    }

    pub async fn send_audio_frame(&self, frame: Vec<i16>) -> Result<(), String> {
        let guard = self.media_session.lock().await;
        if let Some(media) = guard.as_ref() {
            media
                .send_audio_frame(frame)
                .await
                .map_err(|e| e.to_string())
        } else {
            Err("Media session not active".into())
        }
    }

    pub async fn send_dtmf(&self, digit: char) -> Result<(), String> {
        let guard = self.media_session.lock().await;
        if let Some(media) = guard.as_ref() {
            media.send_dtmf(digit).await.map_err(|e| e.to_string())
        } else {
            Err("Media session not active".into())
        }
    }

    pub async fn get_media_stats(&self) -> Option<MediaStats> {
        let guard = self.media_session.lock().await;
        if let Some(media) = guard.as_ref() {
            Some(media.get_stats().await)
        } else {
            None
        }
    }

    pub async fn stop_media(&self) {
        let mut guard = self.media_session.lock().await;
        if let Some(media) = guard.take() {
            let stats = media.stop().await;
            info!(
                packets_sent = stats.packets_sent,
                packets_received = stats.packets_received,
                packets_lost = stats.packets_lost,
                "Call media session terminated"
            );
        }
    }

    /// Constructs an RFC 3261 Section 17.1.1.3 hop-by-hop ACK request for non-2xx final responses.
    pub fn create_non_2xx_ack(invite: &SipRequest, resp: &SipResponse) -> SipRequest {
        let mut headers = HashMap::new();
        if let Some(via) = invite.headers.get("Via") {
            headers.insert("Via".into(), via.clone());
        }
        headers.insert("Max-Forwards".into(), "70".into());
        if let Some(from) = invite.headers.get("From") {
            headers.insert("From".into(), from.clone());
        }
        // Under RFC 3261 Section 17.1.1.3, the To header in ACK MUST match the To header in the response,
        // including any tag assigned by the UAS.
        if let Some(to) = resp.headers.get("To") {
            headers.insert("To".into(), to.clone());
        } else if let Some(to) = invite.headers.get("To") {
            headers.insert("To".into(), to.clone());
        }
        if let Some(call_id) = invite.headers.get("Call-ID") {
            headers.insert("Call-ID".into(), call_id.clone());
        }
        let cseq_num = invite
            .cseq()
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or("1");
        headers.insert("CSeq".into(), format!("{} ACK", cseq_num));
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());

        SipRequest {
            method: SipMethod::Ack,
            ruri: invite.ruri.clone(),
            headers,
            raw_headers: Vec::new(),
            body: String::new(),
        }
    }

    /// Initiates outgoing call sequence with SDP offer
    pub async fn start(&self, destination_addr: SocketAddr) -> Result<(), String> {
        info!(
            sip_call_id = %self.sip_call_id,
            "Initiating outgoing INVITE"
        );

        *self.dest_addr.lock().await = Some(destination_addr);

        let local_ip = self.transport.local_ip_for(destination_addr);
        let local_port = self.transport.local_addr().port();
        let from_tag = generate_tag();
        let branch = generate_branch();
        *self.last_invite_branch.lock().await = Some(branch.clone());

        let cseq = {
            let mut c = self.current_cseq.lock().await;
            let val = *c;
            *c += 1;
            val
        };

        // Construct SDP Offer with G.711 PCMA (8), PCMU (0), and telephone-event (101)
        let sdp_offer = SdpSession::new(local_ip, self.local_rtp_port, vec![8, 0, 101]);
        let sdp_body = sdp_offer.serialize();

        let mut headers = HashMap::new();
        headers.insert(
            "Via".into(),
            format!(
                "SIP/2.0/UDP {}:{};branch={};rport",
                local_ip, local_port, branch
            ),
        );
        headers.insert("Max-Forwards".into(), "70".into());
        headers.insert(
            "From".into(),
            format!("<{}>;tag={}", self.account.aor(), from_tag),
        );
        headers.insert("To".into(), format!("<{}>", self.target_uri));
        headers.insert("Call-ID".into(), self.sip_call_id.clone());
        headers.insert("CSeq".into(), format!("{} INVITE", cseq));
        headers.insert(
            "Contact".into(),
            format!(
                "<sip:{}@{}:{}>",
                self.account.username, local_ip, local_port
            ),
        );
        headers.insert("Content-Type".into(), "application/sdp".into());
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());

        let req = SipRequest {
            method: SipMethod::Invite,
            ruri: self.target_uri.clone(),
            headers,
            raw_headers: Vec::new(),
            body: sdp_body,
        };

        *self.active_invite.lock().await = Some(req.clone());

        let key = TransactionKey {
            branch: branch.clone(),
            method: "INVITE".into(),
        };

        let mut rx = self.transactions.register_client_transaction(key).await;

        let state_sender = self.event_tx.clone();
        let state_mutex = self.state.clone();
        let media_mutex = self.media_session.clone();
        let pcm_out_mutex = self.pcm_out_rx.clone();
        let dtmf_mutex = self.dtmf_rx.clone();
        let transport_for_prov = self.transport.clone();
        let local_rtp_port = self.local_rtp_port;

        let serialized_req = req.serialize();
        let final_resp = match crate::registration::send_and_wait_with_retransmit(
            &self.transport,
            serialized_req.as_bytes(),
            destination_addr,
            &mut rx,
            Duration::from_secs(30),
            move |prov| {
                if prov.status_code == 180 || prov.status_code == 183 {
                    let is_active_or_ringing = if let Ok(mut s) = state_mutex.try_lock() {
                        if matches!(*s, CallState::Dialing) {
                            *s = CallState::RingingRemote;
                            let _ = state_sender.try_send(CallEvent::StateChanged(CallState::RingingRemote));
                        }
                        !matches!(*s, CallState::Terminating { .. } | CallState::Terminated { .. })
                    } else {
                        true
                    };

                    // Early Media (RFC 3960): if provisional response contains SDP, start MediaSession immediately
                    if is_active_or_ringing && !prov.body.is_empty() {
                        if let Ok(remote_sdp) = SdpSession::parse(&prov.body) {
                            info!(
                                payload_types = ?remote_sdp.payload_types,
                                "Early media SDP received in provisional response"
                            );
                            if media_mutex.try_lock().map(|g| g.is_none()).unwrap_or(false) {
                                let dtmf_pt = remote_sdp.dtmf_payload_type();
                                let remote_ip = remote_sdp.connection_ip;
                                let remote_rtp_addr = SocketAddr::new(remote_ip, remote_sdp.audio_port);
                                let local_bind = SocketAddr::new(transport_for_prov.local_addr().ip(), local_rtp_port);
                                let negotiated_codec = if remote_sdp.payload_types.contains(&8) {
                                    AudioCodec::Pcma
                                } else if remote_sdp.payload_types.contains(&0) {
                                    AudioCodec::Pcmu
                                } else {
                                    AudioCodec::Pcma
                                };
                                let media_config = MediaSessionConfig {
                                    local_bind_addr: local_bind,
                                    remote_addr: remote_rtp_addr,
                                    codec: negotiated_codec,
                                    dtmf_payload_type: dtmf_pt,
                                    ssrc: 0x12345678,
                                    target_jitter_depth_packets: 3,
                                    max_jitter_capacity_packets: 100,
                                };
                                let media_mutex_clone = media_mutex.clone();
                                let pcm_clone = pcm_out_mutex.clone();
                                let dtmf_clone = dtmf_mutex.clone();
                                let state_sender_clone = state_sender.clone();
                                tokio::spawn(async move {
                                    if let Ok((ms, pcm_rx, dtmf_rx)) = MediaSession::start(media_config).await {
                                        info!(local_rtp = %ms.local_addr(), "Early media session started");
                                        *media_mutex_clone.lock().await = Some(ms);
                                        *pcm_clone.lock().await = Some(pcm_rx);
                                        *dtmf_clone.lock().await = Some(dtmf_rx);
                                        let _ = state_sender_clone.try_send(CallEvent::EarlyMediaStarted);
                                    }
                                });
                            }
                        }
                    }
                }
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::Timeout,
                    })
                    .await;
                return Err(format!("INVITE request failed: {}", e));
            }
        };

        self.handle_invite_response(final_resp, req, destination_addr)
            .await
    }

    pub async fn setup_media_for_remote_sdp(&self, remote_sdp: &SdpSession) {
        let remote_ip = remote_sdp.connection_ip;
        let remote_rtp_addr = SocketAddr::new(remote_ip, remote_sdp.audio_port);

        {
            let session_guard = self.media_session.lock().await;
            if let Some(ref session) = *session_guard {
                let current_dest = session.destination_addr().await;
                if current_dest != remote_rtp_addr {
                    session.update_destination(remote_rtp_addr).await;
                    info!("Updating RTP MediaSession destination address for remote SDP");
                }
                return;
            }
        }

        info!("Remote SDP negotiated, starting RTP MediaSession");
        let _ = self
            .event_tx
            .send(CallEvent::RemoteSdpNegotiated(remote_sdp.clone()))
            .await;

        let negotiated_codec = if remote_sdp.payload_types.contains(&8) {
            AudioCodec::Pcma
        } else if remote_sdp.payload_types.contains(&0) {
            AudioCodec::Pcmu
        } else {
            AudioCodec::Pcma
        };

        let dtmf_pt = remote_sdp.dtmf_payload_type();

        let local_bind = SocketAddr::new(self.transport.local_addr().ip(), self.local_rtp_port);
        let media_config = MediaSessionConfig {
            local_bind_addr: local_bind,
            remote_addr: remote_rtp_addr,
            codec: negotiated_codec,
            dtmf_payload_type: dtmf_pt,
            ssrc: 0x12345678,
            target_jitter_depth_packets: 3,
            max_jitter_capacity_packets: 100,
        };

        match MediaSession::start(media_config).await {
            Ok((media_sess, pcm_rx, dtmf_rx)) => {
                info!(
                    local_rtp = %media_sess.local_addr(),
                    codec = ?negotiated_codec,
                    dtmf_pt = ?dtmf_pt,
                    "MediaSession started successfully for outgoing call"
                );
                *self.media_session.lock().await = Some(media_sess);
                *self.pcm_out_rx.lock().await = Some(pcm_rx);
                *self.dtmf_rx.lock().await = Some(dtmf_rx);
            }
            Err(e) => {
                warn!("Failed to start MediaSession for outgoing call: {}", e);
            }
        }
    }

    async fn handle_invite_response(
        &self,
        resp: SipResponse,
        initial_req: SipRequest,
        dest_addr: SocketAddr,
    ) -> Result<(), String> {
        match resp.status_code {
            200 => {
                info!("Call answered (200 OK)");
                let was_cancelled = {
                    let st = self.state.lock().await;
                    matches!(
                        *st,
                        CallState::Terminated {
                            reason: DisconnectReason::Cancelled
                        }
                    )
                };

                // Parse remote SDP answer
                if !was_cancelled && !resp.body.is_empty() {
                    if let Ok(remote_sdp) = SdpSession::parse(&resp.body) {
                        info!(
                            payload_types = ?remote_sdp.payload_types,
                            "Remote SDP answer received in 200 OK"
                        );
                        self.setup_media_for_remote_sdp(&remote_sdp).await;
                    }
                }

                // Establish Dialog
                match SipDialog::new_from_outgoing(&initial_req, &resp) {
                    Ok(mut dialog) => {
                        // Send ACK
                        let local_ip = self.transport.local_ip_for(dest_addr);
                        let local_port = self.transport.local_addr().port();
                        let via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);
                        let ack = dialog.create_ack(&via);
                        let _ = self
                            .transport
                            .send_to(ack.serialize().as_bytes(), dest_addr)
                            .await;

                        if was_cancelled {
                            // RFC 3261 Section 9.2: Call was cancelled locally before 200 OK arrived.
                            // Must establish session (ACK sent above) and immediately terminate with BYE.
                            info!(
                                "200 OK received for cancelled call; terminating immediately with BYE (RFC 3261 Section 9.2)"
                            );
                            let bye = dialog.create_bye(&via);
                            let _ = self
                                .transport
                                .send_to(bye.serialize().as_bytes(), dest_addr)
                                .await;
                            self.stop_media().await;
                            return Ok(());
                        }

                        *self.dialog.lock().await = Some(dialog);
                        let _ = self
                            .transition_state(CallState::Active {
                                started_at: chrono::Utc::now(),
                            })
                            .await;
                        Ok(())
                    }
                    Err(e) => {
                        let err = format!("Failed to establish SIP dialog: {:?}", e);
                        error!("{}", err);
                        Err(err)
                    }
                }
            }
            401 | 407 => {
                debug!("Received challenge for INVITE, computing Digest authorization");
                // Hop-by-hop ACK for challenged transaction (RFC 3261 Section 17.1.1.3)
                let ack = Self::create_non_2xx_ack(&initial_req, &resp);
                let _ = self
                    .transport
                    .send_to(ack.serialize().as_bytes(), dest_addr)
                    .await;

                let challenge_header = resp
                    .headers
                    .get("WWW-Authenticate")
                    .or_else(|| resp.headers.get("Proxy-Authenticate"))
                    .ok_or_else(|| "Missing challenge header".to_string())?;

                let challenge = DigestChallenge::parse(challenge_header)
                    .map_err(|e| format!("Failed to parse challenge: {:?}", e))?;

                let auth_header = DigestAuthorizer::build_authorization_header(
                    &self.account.username,
                    &self.password,
                    &challenge,
                    "INVITE",
                    &self.target_uri,
                    1,
                );

                // Resend authenticated INVITE with new CSeq & Via branch
                let cseq2 = {
                    let mut c = self.current_cseq.lock().await;
                    let val = *c;
                    *c += 1;
                    val
                };
                let branch2 = generate_branch();
                *self.last_invite_branch.lock().await = Some(branch2.clone());

                let local_ip = self.transport.local_ip_for(dest_addr);
                let local_port = self.transport.local_addr().port();
                let mut headers2 = initial_req.headers.clone();
                headers2.insert(
                    "Via".into(),
                    format!(
                        "SIP/2.0/UDP {}:{};branch={};rport",
                        local_ip, local_port, branch2
                    ),
                );
                headers2.insert("CSeq".into(), format!("{} INVITE", cseq2));
                if resp.status_code == 407 {
                    headers2.insert("Proxy-Authorization".into(), auth_header);
                } else {
                    headers2.insert("Authorization".into(), auth_header);
                }

                let req2 = SipRequest {
                    method: SipMethod::Invite,
                    ruri: self.target_uri.clone(),
                    headers: headers2,
                    raw_headers: Vec::new(),
                    body: initial_req.body.clone(),
                };

                *self.active_invite.lock().await = Some(req2.clone());

                let key2 = TransactionKey {
                    branch: branch2,
                    method: "INVITE".into(),
                };
                let mut rx2 = self.transactions.register_client_transaction(key2).await;

                let state_sender2 = self.event_tx.clone();
                let state_mutex2 = self.state.clone();
                let media_mutex2 = self.media_session.clone();
                let pcm_out_mutex2 = self.pcm_out_rx.clone();
                let dtmf_mutex2 = self.dtmf_rx.clone();
                let transport_for_prov2 = self.transport.clone();
                let local_rtp_port2 = self.local_rtp_port;

                let serialized_req2 = req2.serialize();
                let final_resp2 = match crate::registration::send_and_wait_with_retransmit(
                    &self.transport,
                    serialized_req2.as_bytes(),
                    dest_addr,
                    &mut rx2,
                    Duration::from_secs(30),
                    move |prov| {
                        if prov.status_code == 180 || prov.status_code == 183 {
                            let is_active_or_ringing = if let Ok(mut s) = state_mutex2.try_lock() {
                                if matches!(*s, CallState::Dialing) {
                                    *s = CallState::RingingRemote;
                                    let _ = state_sender2.try_send(CallEvent::StateChanged(CallState::RingingRemote));
                                }
                                !matches!(*s, CallState::Terminating { .. } | CallState::Terminated { .. })
                            } else {
                                true
                            };

                            if is_active_or_ringing && !prov.body.is_empty() {
                                if let Ok(remote_sdp) = SdpSession::parse(&prov.body) {
                                    if media_mutex2.try_lock().map(|g| g.is_none()).unwrap_or(false) {
                                        let dtmf_pt = remote_sdp.dtmf_payload_type();
                                        let remote_ip = remote_sdp.connection_ip;
                                        let remote_rtp_addr = SocketAddr::new(remote_ip, remote_sdp.audio_port);
                                        let local_bind = SocketAddr::new(transport_for_prov2.local_addr().ip(), local_rtp_port2);
                                        let negotiated_codec = if remote_sdp.payload_types.contains(&8) {
                                            AudioCodec::Pcma
                                        } else if remote_sdp.payload_types.contains(&0) {
                                            AudioCodec::Pcmu
                                        } else {
                                            AudioCodec::Pcma
                                        };
                                        let media_config = MediaSessionConfig {
                                            local_bind_addr: local_bind,
                                            remote_addr: remote_rtp_addr,
                                            codec: negotiated_codec,
                                            dtmf_payload_type: dtmf_pt,
                                            ssrc: 0x12345678,
                                            target_jitter_depth_packets: 3,
                                            max_jitter_capacity_packets: 100,
                                        };
                                        let media_mutex_clone = media_mutex2.clone();
                                        let pcm_clone = pcm_out_mutex2.clone();
                                        let dtmf_clone = dtmf_mutex2.clone();
                                        let state_sender_clone = state_sender2.clone();
                                        tokio::spawn(async move {
                                            if let Ok((ms, pcm_rx, dtmf_rx)) = MediaSession::start(media_config).await {
                                                info!(local_rtp = %ms.local_addr(), "Early media session started");
                                                *media_mutex_clone.lock().await = Some(ms);
                                                *pcm_clone.lock().await = Some(pcm_rx);
                                                *dtmf_clone.lock().await = Some(dtmf_rx);
                                                let _ = state_sender_clone.try_send(CallEvent::EarlyMediaStarted);
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    },
                )
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        self.stop_media().await;
                        let _ = self
                            .transition_state(CallState::Terminated {
                                reason: DisconnectReason::Timeout,
                            })
                            .await;
                        return Err(format!("Authenticated INVITE failed: {}", e));
                    }
                };

                // Recurse to handle final response (e.g. 200 OK or error)
                Box::pin(self.handle_invite_response(final_resp2, req2, dest_addr)).await
            }
            487 => {
                warn!("Request terminated (487 Request Terminated / Cancelled)");
                let ack = Self::create_non_2xx_ack(&initial_req, &resp);
                let _ = self
                    .transport
                    .send_to(ack.serialize().as_bytes(), dest_addr)
                    .await;

                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::Cancelled,
                    })
                    .await;
                Err("Call was cancelled".into())
            }
            486 => {
                warn!("Remote party busy (486 Busy Here)");
                let ack = Self::create_non_2xx_ack(&initial_req, &resp);
                let _ = self
                    .transport
                    .send_to(ack.serialize().as_bytes(), dest_addr)
                    .await;

                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::Busy,
                    })
                    .await;
                Err("Destination is busy".into())
            }
            603 | 403 => {
                warn!("Call rejected by remote party: {}", resp.status_code);
                let ack = Self::create_non_2xx_ack(&initial_req, &resp);
                let _ = self
                    .transport
                    .send_to(ack.serialize().as_bytes(), dest_addr)
                    .await;

                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::Rejected,
                    })
                    .await;
                Err("Call was rejected".into())
            }
            404 | 480 => {
                warn!("Destination unreachable: {}", resp.status_code);
                let ack = Self::create_non_2xx_ack(&initial_req, &resp);
                let _ = self
                    .transport
                    .send_to(ack.serialize().as_bytes(), dest_addr)
                    .await;

                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::Unreachable,
                    })
                    .await;
                Err("Destination unreachable".into())
            }
            other => {
                let ack = Self::create_non_2xx_ack(&initial_req, &resp);
                let _ = self
                    .transport
                    .send_to(ack.serialize().as_bytes(), dest_addr)
                    .await;

                let reason = format!("Call failed with status {} {}", other, resp.reason_phrase);
                warn!("{}", reason);
                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::NetworkError(reason.clone()),
                    })
                    .await;
                Err(reason)
            }
        }
    }

    /// Hang up an active call by issuing BYE to a specific address (RFC 3261 Section 15)
    pub async fn hangup(&self, dest_addr: SocketAddr) -> Result<(), String> {
        let mut dialog_guard = self.dialog.lock().await;
        if let Some(ref mut dialog) = *dialog_guard {
            info!("Sending in-dialog BYE");
            let local_ip = self.transport.local_ip_for(dest_addr);
            let local_port = self.transport.local_addr().port();
            let via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);
            let bye = dialog.create_bye(&via);
            let _ = self
                .transport
                .send_to(bye.serialize().as_bytes(), dest_addr)
                .await;
            *dialog_guard = None;
        }

        self.stop_media().await;

        let _ = self
            .transition_state(CallState::Terminated {
                reason: DisconnectReason::LocalHangup,
            })
            .await;
        Ok(())
    }

    /// Hang up active call using the remembered destination address
    pub async fn hangup_current(&self) -> Result<(), String> {
        let target_addr = match *self.dest_addr.lock().await {
            Some(a) => a,
            None => return Err("Unknown destination address for BYE".into()),
        };
        self.hangup(target_addr).await
    }

    /// Places active call on hold via in-dialog re-INVITE (RFC 3264 a=sendonly)
    pub async fn hold(&self) -> Result<(), String> {
        {
            let current_state = self.state.lock().await;
            if !matches!(*current_state, CallState::Active { .. }) {
                return Err("Cannot hold call that is not Active".into());
            }
        }

        let dest_addr = match *self.dest_addr.lock().await {
            Some(a) => a,
            None => return Err("No destination address for hold".into()),
        };

        let mut dialog_guard = self.dialog.lock().await;
        let dialog = dialog_guard
            .as_mut()
            .ok_or_else(|| "No active dialog for hold".to_string())?;

        let local_ip = self.transport.local_ip_for(dest_addr);
        let local_port = self.transport.local_addr().port();
        let local_via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);
        let contact = format!(
            "<sip:{}@{}:{}>",
            self.account.username, local_ip, local_port
        );

        let sdp = SdpSession::new(local_ip, self.local_rtp_port, vec![8, 0, 101])
            .with_direction(sip_protocol::MediaDirection::SendOnly);
        let sdp_body = sdp.serialize();

        let reinvite = dialog.create_reinvite(&local_via, &contact, &sdp_body);
        let branch = reinvite
            .headers
            .get("Via")
            .and_then(|v| sip_protocol::extract_branch(v))
            .unwrap_or_else(sip_protocol::generate_branch);
        let key = TransactionKey {
            branch,
            method: "INVITE".into(),
        };

        let mut rx = self.transactions.register_client_transaction(key).await;
        let serialized = reinvite.serialize();
        self.transport
            .send_to(serialized.as_bytes(), dest_addr)
            .await
            .map_err(|e| format!("Transport error on re-INVITE: {}", e))?;
        drop(dialog_guard);

        let _resp = match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(r)) if r.status_code == 200 => r,
            Ok(Some(r)) => return Err(format!("re-INVITE rejected with status {}", r.status_code)),
            _ => return Err("re-INVITE timed out".into()),
        };

        let media_guard = self.media_session.lock().await;
        if let Some(ref media) = *media_guard {
            media.set_muted(true);
        }

        let dialog_guard = self.dialog.lock().await;
        if let Some(ref d) = *dialog_guard {
            let ack = d.create_ack(&local_via);
            let _ = self
                .transport
                .send_to(ack.serialize().as_bytes(), dest_addr)
                .await;
        }

        self.transition_state(CallState::Held).await;
        info!(call_id = %self.call_id, "Call successfully placed on hold");
        Ok(())
    }

    /// Resumes active call from hold via in-dialog re-INVITE (RFC 3264 a=sendrecv)
    pub async fn resume(&self) -> Result<(), String> {
        {
            let current_state = self.state.lock().await;
            if !matches!(*current_state, CallState::Held) {
                return Err("Cannot resume call that is not Held".into());
            }
        }

        let dest_addr = match *self.dest_addr.lock().await {
            Some(a) => a,
            None => return Err("No destination address for resume".into()),
        };

        let mut dialog_guard = self.dialog.lock().await;
        let dialog = dialog_guard
            .as_mut()
            .ok_or_else(|| "No active dialog for resume".to_string())?;

        let local_ip = self.transport.local_ip_for(dest_addr);
        let local_port = self.transport.local_addr().port();
        let local_via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);
        let contact = format!(
            "<sip:{}@{}:{}>",
            self.account.username, local_ip, local_port
        );

        let sdp = SdpSession::new(local_ip, self.local_rtp_port, vec![8, 0, 101])
            .with_direction(sip_protocol::MediaDirection::SendRecv);
        let sdp_body = sdp.serialize();

        let reinvite = dialog.create_reinvite(&local_via, &contact, &sdp_body);
        let branch = reinvite
            .headers
            .get("Via")
            .and_then(|v| sip_protocol::extract_branch(v))
            .unwrap_or_else(sip_protocol::generate_branch);
        let key = TransactionKey {
            branch,
            method: "INVITE".into(),
        };

        let mut rx = self.transactions.register_client_transaction(key).await;
        let serialized = reinvite.serialize();
        self.transport
            .send_to(serialized.as_bytes(), dest_addr)
            .await
            .map_err(|e| format!("Transport error on re-INVITE resume: {}", e))?;
        drop(dialog_guard);

        let _resp = match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(r)) if r.status_code == 200 => r,
            Ok(Some(r)) => {
                return Err(format!(
                    "re-INVITE resume rejected with status {}",
                    r.status_code
                ));
            }
            _ => return Err("re-INVITE resume timed out".into()),
        };

        let media_guard = self.media_session.lock().await;
        if let Some(ref media) = *media_guard {
            media.set_muted(false);
        }

        let dialog_guard = self.dialog.lock().await;
        if let Some(ref d) = *dialog_guard {
            let ack = d.create_ack(&local_via);
            let _ = self
                .transport
                .send_to(ack.serialize().as_bytes(), dest_addr)
                .await;
        }

        self.transition_state(CallState::Active {
            started_at: chrono::Utc::now(),
        })
        .await;
        info!(call_id = %self.call_id, "Call successfully resumed from hold");
        Ok(())
    }

    /// Cancels an uncompleted call before 2xx answer to a specific address (RFC 3261 Section 9)
    pub async fn cancel(&self, dest_addr: SocketAddr) -> Result<(), String> {
        self.stop_media().await;

        let invite_opt = self.active_invite.lock().await.clone();
        if let Some(invite) = invite_opt {
            let cseq_num = invite
                .cseq()
                .and_then(|c| c.split_whitespace().next())
                .unwrap_or("1");

            info!(cseq = %cseq_num, "Sending CANCEL for pending INVITE");

            let mut headers = HashMap::new();
            if let Some(via) = invite.headers.get("Via") {
                headers.insert("Via".into(), via.clone());
            }
            headers.insert("Max-Forwards".into(), "70".into());
            if let Some(from) = invite.headers.get("From") {
                // Must preserve From tag (RFC 3261 Section 9.1)
                headers.insert("From".into(), from.clone());
            }
            if let Some(to) = invite.headers.get("To") {
                headers.insert("To".into(), to.clone());
            }
            if let Some(call_id) = invite.headers.get("Call-ID") {
                headers.insert("Call-ID".into(), call_id.clone());
            }
            headers.insert("CSeq".into(), format!("{} CANCEL", cseq_num));
            headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());

            let cancel_req = SipRequest {
                method: SipMethod::Cancel,
                ruri: invite.ruri.clone(),
                headers,
                raw_headers: Vec::new(),
                body: String::new(),
            };

            let _ = self
                .transport
                .send_to(cancel_req.serialize().as_bytes(), dest_addr)
                .await;
        }

        let _ = self
            .transition_state(CallState::Terminated {
                reason: DisconnectReason::Cancelled,
            })
            .await;
        Ok(())
    }

    /// Cancels uncompleted call using the remembered destination address
    pub async fn cancel_current(&self) -> Result<(), String> {
        let target_addr = match *self.dest_addr.lock().await {
            Some(a) => a,
            None => return Err("Unknown destination address for CANCEL".into()),
        };
        self.cancel(target_addr).await
    }

    /// Handles an incoming in-dialog BYE request from the remote party (Remote Hangup)
    pub async fn handle_incoming_bye(
        &self,
        req: &SipRequest,
        src: SocketAddr,
    ) -> Option<SipResponse> {
        let mut dialog_guard = self.dialog.lock().await;
        if let Some(ref dialog) = *dialog_guard {
            if dialog.matches_incoming_request(req) {
                info!(call_id = %self.call_id, "Handling remote BYE in active dialog");
                let resp = dialog.create_response_to_request(req, 200, "OK");
                let _ = self
                    .transport
                    .send_to(resp.serialize().as_bytes(), src)
                    .await;
                *dialog_guard = None;

                self.stop_media().await;

                let _ = self
                    .transition_state(CallState::Terminated {
                        reason: DisconnectReason::RemoteHangup,
                    })
                    .await;
                return Some(resp);
            }
        }
        drop(dialog_guard);

        // Absorb retransmitted or delayed BYE if call is already terminated (RFC 3261 Section 15.1.2)
        let st = self.state.lock().await;
        if matches!(*st, CallState::Terminated { .. })
            && req
                .call_id()
                .map(|id| id == self.sip_call_id)
                .unwrap_or(false)
        {
            let ok_bye = SipResponse::create_for_request(req, 200, "OK", None, None, None);
            let _ = self
                .transport
                .send_to(ok_bye.serialize().as_bytes(), src)
                .await;
            return Some(ok_bye);
        }

        None
    }

    /// Handles an incoming in-dialog re-INVITE request (RFC 3261 Section 14 / RFC 3264)
    pub async fn handle_reinvite(&self, req: &SipRequest, src: SocketAddr) -> bool {
        let mut dialog_guard = self.dialog.lock().await;
        let dialog = match dialog_guard.as_mut() {
            Some(d) if d.matches_incoming_request(req) => d,
            _ => return false,
        };

        info!(call_id = %self.call_id, "Handling in-dialog re-INVITE in active dialog");

        let offer_sdp = SdpSession::parse(&req.body).ok();
        let (answer_direction, is_held) = match offer_sdp.as_ref().map(|s| s.direction) {
            Some(sip_protocol::MediaDirection::SendOnly) => {
                (sip_protocol::MediaDirection::RecvOnly, true)
            }
            Some(sip_protocol::MediaDirection::Inactive) => {
                (sip_protocol::MediaDirection::Inactive, true)
            }
            Some(sip_protocol::MediaDirection::RecvOnly) => {
                (sip_protocol::MediaDirection::SendOnly, false)
            }
            _ => (sip_protocol::MediaDirection::SendRecv, false),
        };

        let bound_rtp_port = {
            let media_guard = self.media_session.lock().await;
            if let Some(ref media) = *media_guard {
                if let Some(offer) = offer_sdp.as_ref() {
                    let new_dest = SocketAddr::new(offer.connection_ip, offer.audio_port);
                    media.update_destination(new_dest).await;
                }
                media.set_muted(is_held);
                media.local_addr().port()
            } else {
                self.local_rtp_port
            }
        };

        let local_ip = self.transport.local_ip_for(src);
        let answer_sdp = SdpSession::new(local_ip, bound_rtp_port, vec![8, 0, 101])
            .with_direction(answer_direction);
        let sdp_body = answer_sdp.serialize();

        let contact = format!(
            "<sip:{}@{}:{}>",
            self.account.username,
            local_ip,
            self.transport.local_addr().port()
        );

        let resp = dialog.create_response_with_body(
            req,
            200,
            "OK",
            "application/sdp",
            &sdp_body,
            Some(&contact),
        );
        let _ = self
            .transport
            .send_to(resp.serialize().as_bytes(), src)
            .await;
        drop(dialog_guard);

        if is_held {
            let _ = self.transition_state(CallState::Held).await;
        } else {
            let is_currently_held = {
                let st = self.state.lock().await;
                matches!(*st, CallState::Held)
            };
            if is_currently_held {
                let _ = self
                    .transition_state(CallState::Active {
                        started_at: chrono::Utc::now(),
                    })
                    .await;
            }
        }

        true
    }

    pub async fn transition_state(&self, new_state: CallState) -> bool {
        let mut state = self.state.lock().await;
        // Terminal state is final: no transitions permitted once Terminated
        if matches!(*state, CallState::Terminated { .. }) {
            return false;
        }
        // Deduplicate identical state transitions
        if *state == new_state {
            return false;
        }
        *state = new_state.clone();
        let _ = self.event_tx.send(CallEvent::StateChanged(new_state)).await;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_non_2xx_ack_rfc3261_compliance() {
        let mut inv_headers = HashMap::new();
        inv_headers.insert(
            "Via".into(),
            "SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK-inv-branch-123;rport".into(),
        );
        inv_headers.insert(
            "From".into(),
            "<sip:alice@example.com>;tag=alice-tag-777".into(),
        );
        inv_headers.insert("To".into(), "<sip:bob@example.com>".into());
        inv_headers.insert("Call-ID".into(), "call-id-998877".into());
        inv_headers.insert("CSeq".into(), "3 INVITE".into());

        let invite = SipRequest {
            method: SipMethod::Invite,
            ruri: "sip:bob@example.com".into(),
            headers: inv_headers,
            raw_headers: Vec::new(),
            body: "".into(),
        };

        let mut resp_headers = HashMap::new();
        resp_headers.insert(
            "Via".into(),
            "SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK-inv-branch-123;rport".into(),
        );
        resp_headers.insert(
            "From".into(),
            "<sip:alice@example.com>;tag=alice-tag-777".into(),
        );
        resp_headers.insert("To".into(), "<sip:bob@example.com>;tag=bob-tag-999".into());
        resp_headers.insert("Call-ID".into(), "call-id-998877".into());
        resp_headers.insert("CSeq".into(), "3 INVITE".into());

        let resp_486 = SipResponse {
            status_code: 486,
            reason_phrase: "Busy Here".into(),
            headers: resp_headers,
            raw_headers: Vec::new(),
            body: "".into(),
        };

        let ack = OutgoingCallSession::create_non_2xx_ack(&invite, &resp_486);
        assert_eq!(ack.method, SipMethod::Ack);
        assert_eq!(ack.ruri, "sip:bob@example.com");
        assert_eq!(
            ack.headers.get("Via").unwrap(),
            "SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK-inv-branch-123;rport"
        );
        assert_eq!(
            ack.headers.get("From").unwrap(),
            "<sip:alice@example.com>;tag=alice-tag-777"
        );
        // Must contain To tag from UAS response per RFC 3261 Section 17.1.1.3
        assert_eq!(
            ack.headers.get("To").unwrap(),
            "<sip:bob@example.com>;tag=bob-tag-999"
        );
        assert_eq!(ack.headers.get("Call-ID").unwrap(), "call-id-998877");
        assert_eq!(ack.headers.get("CSeq").unwrap(), "3 ACK");
    }
}
