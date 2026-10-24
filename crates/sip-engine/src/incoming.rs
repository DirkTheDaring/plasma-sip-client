//! Incoming SIP Call Session managing inbound INVITE, 100 Trying, 180 Ringing, 200 OK, Reject, Cancel, and BYE.

use chrono::Utc;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::info;

use crate::call::CallEvent;
use sip_domain::{AudioCodec, CallId, CallState, DisconnectReason};
use sip_media::{MediaSession, MediaSessionConfig, dtmf::DtmfEvent, stats::MediaStats};
use sip_protocol::{SdpSession, SipDialog, SipRequest, SipResponse, generate_tag};
use sip_transport::SipUdpTransport;

pub struct IncomingCallSession {
    pub call_id: CallId,
    pub sip_call_id: String,
    pub caller_identity: String,
    pub remote_sdp: SdpSession,
    pub invite_request: SipRequest,
    pub to_tag: String,
    pub transport: Arc<SipUdpTransport>,
    pub transactions: Arc<sip_protocol::TransactionManager>,
    pub remote_addr: SocketAddr,
    pub state: Arc<Mutex<CallState>>,
    pub dialog: Arc<Mutex<Option<SipDialog>>>,
    pub media_session: Arc<Mutex<Option<MediaSession>>>,
    pub pcm_out_rx: Arc<Mutex<Option<mpsc::Receiver<Vec<i16>>>>>,
    pub dtmf_rx: Arc<Mutex<Option<mpsc::Receiver<DtmfEvent>>>>,
    event_tx: mpsc::Sender<CallEvent>,
}

fn extract_callee_user(req: &SipRequest) -> &str {
    if let Some(stripped) = req.ruri.strip_prefix("sip:") {
        if let Some((user, _host)) = stripped.split_once('@') {
            return user;
        }
    }
    if let Some(to) = req.headers.get("To") {
        if let Some(start) = to.find("sip:") {
            let rest = &to[start + 4..];
            if let Some((user, _)) = rest.split_once('@') {
                return user;
            }
        }
    }
    "plasma-sip"
}

impl IncomingCallSession {
    /// Creates and initializes an incoming call session from an inbound INVITE request.
    /// Immediately replies with 100 Trying and 180 Ringing.
    pub async fn new_and_ring(
        invite_request: SipRequest,
        remote_addr: SocketAddr,
        transport: Arc<SipUdpTransport>,
        event_tx: mpsc::Sender<CallEvent>,
    ) -> Result<Self, String> {
        Self::with_call_id_and_ring(
            CallId::new(),
            invite_request,
            remote_addr,
            transport,
            event_tx,
        )
        .await
    }

    pub fn with_transactions(
        mut self,
        transactions: Arc<sip_protocol::TransactionManager>,
    ) -> Self {
        self.transactions = transactions;
        self
    }

    pub async fn with_call_id_and_ring(
        call_id: CallId,
        invite_request: SipRequest,
        remote_addr: SocketAddr,
        transport: Arc<SipUdpTransport>,
        event_tx: mpsc::Sender<CallEvent>,
    ) -> Result<Self, String> {
        let sip_call_id = invite_request
            .call_id()
            .ok_or_else(|| "Missing Call-ID in incoming INVITE".to_string())?
            .to_string();

        let caller_party = invite_request.caller_info();
        let caller_identity = caller_party.display_string.clone();

        let remote_sdp = SdpSession::parse(&invite_request.body)
            .map_err(|e| format!("Invalid SDP in incoming INVITE: {}", e))?;

        let to_tag = generate_tag();

        let session = Self {
            call_id,
            sip_call_id,
            caller_identity,
            remote_sdp,
            invite_request: invite_request.clone(),
            to_tag: to_tag.clone(),
            transport: Arc::clone(&transport),
            transactions: Arc::new(sip_protocol::TransactionManager::new()),
            remote_addr,
            state: Arc::new(Mutex::new(CallState::RingingLocal)),
            dialog: Arc::new(Mutex::new(None)),
            media_session: Arc::new(Mutex::new(None)),
            pcm_out_rx: Arc::new(Mutex::new(None)),
            dtmf_rx: Arc::new(Mutex::new(None)),
            event_tx,
        };

        // 1. Send 100 Trying
        let trying =
            SipResponse::create_for_request(&invite_request, 100, "Trying", None, None, None);
        let _ = transport
            .send_to(trying.serialize().as_bytes(), remote_addr)
            .await;

        let local_ip = transport.local_ip_for(remote_addr);
        let local_port = transport.local_addr().port();
        let callee_user = extract_callee_user(&invite_request);
        let contact_hdr = format!("<sip:{}@{}:{}>", callee_user, local_ip, local_port);

        // 2. Send 180 Ringing with local to_tag and Contact header (RFC 3261 Section 12.1.1)
        let ringing = SipResponse::create_for_request(
            &invite_request,
            180,
            "Ringing",
            Some(&to_tag),
            Some(&contact_hdr),
            None,
        );
        transport
            .send_to(ringing.serialize().as_bytes(), remote_addr)
            .await
            .map_err(|e| format!("Failed to send 180 Ringing: {}", e))?;

        info!(
            call_id = %session.call_id,
            sip_call_id = %session.sip_call_id,
            "Incoming call received and ringing"
        );

        let _ = session
            .event_tx
            .send(CallEvent::StateChanged(CallState::RingingLocal))
            .await;

        Ok(session)
    }

    pub async fn get_state(&self) -> CallState {
        self.state.lock().await.clone()
    }

    /// Answers the incoming call: builds SDP answer, sends 200 OK, establishes dialog, and launches RTP media session.
    pub async fn answer(&self, local_rtp_port: u16, codec: AudioCodec) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if *st != CallState::RingingLocal {
            return Err(format!("Cannot answer call in state {:?}", *st));
        }

        let local_addr = self.transport.local_addr();

        // 1. Start RTP Media Session
        let remote_rtp_addr =
            SocketAddr::new(self.remote_sdp.connection_ip, self.remote_sdp.audio_port);

        let media_config = MediaSessionConfig {
            local_bind_addr: SocketAddr::new(local_addr.ip(), local_rtp_port),
            remote_addr: remote_rtp_addr,
            codec,
            dtmf_payload_type: Some(101),
            ssrc: 0x2468ACE0,
            target_jitter_depth_packets: 3,
            max_jitter_capacity_packets: 100,
        };

        let (media_sess, pcm_out_rx, dtmf_rx) = MediaSession::start(media_config)
            .await
            .map_err(|e| format!("Failed to start RTP media session: {}", e))?;

        let bound_rtp_port = media_sess.local_addr().port();
        *self.media_session.lock().await = Some(media_sess);
        *self.pcm_out_rx.lock().await = Some(pcm_out_rx);
        *self.dtmf_rx.lock().await = Some(dtmf_rx);

        // 2. Build SDP answer using actual bound RTP port
        let payload_type = match codec {
            AudioCodec::Pcma => 8,
            AudioCodec::Pcmu => 0,
            _ => 8,
        };

        let local_ip = self.transport.local_ip_for(self.remote_addr);
        let sdp_answer = SdpSession::new(
            local_ip,
            bound_rtp_port,
            vec![payload_type, 101], // Audio codec and RFC 4733 telephone-event
        );
        let sdp_body = sdp_answer.serialize();

        let contact_hdr = format!(
            "<sip:{}@{}:{}>",
            extract_callee_user(&self.invite_request),
            local_ip,
            local_addr.port()
        );
        let ok_res = SipResponse::create_for_request(
            &self.invite_request,
            200,
            "OK",
            Some(&self.to_tag),
            Some(&contact_hdr),
            Some(sdp_body),
        );

        self.transport
            .send_to(ok_res.serialize().as_bytes(), self.remote_addr)
            .await
            .map_err(|e| format!("Failed to send 200 OK: {}", e))?;

        // 3. Establish dialog
        let dialog = SipDialog::new_from_incoming(&self.invite_request, &ok_res)
            .map_err(|e| format!("Failed to establish dialog: {}", e))?;

        *self.dialog.lock().await = Some(dialog);

        let new_state = CallState::Active {
            started_at: Utc::now(),
        };
        *st = new_state.clone();
        let _ = self.event_tx.send(CallEvent::StateChanged(new_state)).await;

        info!(call_id = %self.call_id, "Incoming call answered and active");
        Ok(())
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

    /// Rejects the incoming call (e.g. 603 Decline or 486 Busy Here).
    pub async fn reject(&self, status_code: u16, reason: &str) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if *st != CallState::RingingLocal {
            return Err(format!("Cannot reject call in state {:?}", *st));
        }

        let mut res = SipResponse::create_for_request(
            &self.invite_request,
            status_code,
            reason,
            Some(&self.to_tag),
            None,
            None,
        );

        // RFC 3326 Reason header: standard Q.850 release causes for carrier/PBX interworking
        if status_code == 603 {
            res.add_header("Reason", "Q.850;cause=21;text=\"Call Rejected\"");
        } else if status_code == 486 || status_code == 600 {
            res.add_header("Reason", "Q.850;cause=17;text=\"User Busy\"");
        }

        let _ = self
            .transport
            .send_to(res.serialize().as_bytes(), self.remote_addr)
            .await;

        let disconnect_reason = if status_code == 486 || status_code == 600 {
            DisconnectReason::Busy
        } else {
            DisconnectReason::Rejected
        };

        *st = CallState::Terminated {
            reason: disconnect_reason,
        };
        let _ = self
            .event_tx
            .send(CallEvent::StateChanged(st.clone()))
            .await;

        info!(call_id = %self.call_id, status = status_code, "Incoming call rejected");
        Ok(())
    }

    /// Forcefully answer and immediately drop the call with BYE to cancel all other forks on PBX.
    pub async fn answer_and_drop(&self) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if *st != CallState::RingingLocal {
            return Err(format!("Cannot answer_and_drop call in state {:?}", *st));
        }

        let local_addr = self.transport.local_addr();
        let local_ip = self.transport.local_ip_for(self.remote_addr);

        let sdp_answer = SdpSession::new(local_ip, local_addr.port() + 10, vec![8, 0, 101]);
        let sdp_body = sdp_answer.serialize();
        let contact_hdr = format!(
            "<sip:{}@{}:{}>",
            extract_callee_user(&self.invite_request),
            local_ip,
            local_addr.port()
        );

        let ok_res = SipResponse::create_for_request(
            &self.invite_request,
            200,
            "OK",
            Some(&self.to_tag),
            Some(&contact_hdr),
            Some(sdp_body),
        );

        self.transport
            .send_to(ok_res.serialize().as_bytes(), self.remote_addr)
            .await
            .map_err(|e| format!("Failed to send 200 OK: {}", e))?;

        let mut dialog = SipDialog::new_from_incoming(&self.invite_request, &ok_res)
            .map_err(|e| format!("Failed to establish dialog: {}", e))?;

        let local_via = format!("SIP/2.0/UDP {}:{}", local_ip, local_addr.port());
        let bye_req = dialog.create_bye(&local_via);

        let transport = self.transport.clone();
        let remote_addr = self.remote_addr;

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = transport
                .send_to(bye_req.serialize().as_bytes(), remote_addr)
                .await;
        });

        *st = CallState::Terminated {
            reason: DisconnectReason::Rejected,
        };
        let _ = self
            .event_tx
            .send(CallEvent::StateChanged(st.clone()))
            .await;

        info!(call_id = %self.call_id, "Incoming call answered and dropped forcefully");
        Ok(())
    }

    /// Handles an incoming CANCEL request from remote party before answer.
    pub async fn handle_cancel(&self, cancel_req: &SipRequest) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if matches!(*st, CallState::Terminated { .. }) {
            // Already terminated: absorb retransmitted CANCEL with 200 OK (RFC 3261 Section 9.2)
            let ok_cancel =
                SipResponse::create_for_request(cancel_req, 200, "OK", None, None, None);
            let _ = self
                .transport
                .send_to(ok_cancel.serialize().as_bytes(), self.remote_addr)
                .await;
            return Ok(());
        }

        if *st != CallState::RingingLocal {
            return Err("Cannot handle CANCEL when not in RingingLocal".into());
        }

        // 1. Reply 200 OK to the CANCEL request
        let ok_cancel = SipResponse::create_for_request(cancel_req, 200, "OK", None, None, None);
        let _ = self
            .transport
            .send_to(ok_cancel.serialize().as_bytes(), self.remote_addr)
            .await;

        // 2. Reply 487 Request Terminated to the original INVITE request
        let term_res = SipResponse::create_for_request(
            &self.invite_request,
            487,
            "Request Terminated",
            Some(&self.to_tag),
            None,
            None,
        );
        let _ = self
            .transport
            .send_to(term_res.serialize().as_bytes(), self.remote_addr)
            .await;

        *st = CallState::Terminated {
            reason: DisconnectReason::Cancelled,
        };
        let _ = self
            .event_tx
            .send(CallEvent::StateChanged(st.clone()))
            .await;

        info!(call_id = %self.call_id, "Incoming call cancelled by remote party");
        Ok(())
    }

    /// Handles remote BYE request when active or held.
    pub async fn handle_bye(&self, bye_req: &SipRequest) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if matches!(*st, CallState::Terminated { .. }) {
            // Already terminated: absorb retransmitted or delayed BYE with 200 OK (RFC 3261 Section 15.1.2)
            let ok_bye = SipResponse::create_for_request(bye_req, 200, "OK", None, None, None);
            let _ = self
                .transport
                .send_to(ok_bye.serialize().as_bytes(), self.remote_addr)
                .await;
            return Ok(());
        }

        if !matches!(*st, CallState::Active { .. } | CallState::Held) {
            return Err("Cannot handle BYE when not Active or Held".into());
        }

        let ok_bye = SipResponse::create_for_request(bye_req, 200, "OK", None, None, None);
        let _ = self
            .transport
            .send_to(ok_bye.serialize().as_bytes(), self.remote_addr)
            .await;

        if let Some(media) = self.media_session.lock().await.take() {
            let _ = media.stop().await;
        }

        *self.dialog.lock().await = None;

        *st = CallState::Terminated {
            reason: DisconnectReason::RemoteHangup,
        };
        let _ = self
            .event_tx
            .send(CallEvent::StateChanged(st.clone()))
            .await;

        info!(call_id = %self.call_id, "Incoming call hung up by remote party");
        Ok(())
    }

    /// Local hangup when active or held.
    pub async fn hangup(&self) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if !matches!(*st, CallState::Active { .. } | CallState::Held) {
            return Err("Cannot hangup when not Active or Held".into());
        }

        let local_ip = self.transport.local_ip_for(self.remote_addr);
        let local_port = self.transport.local_addr().port();
        let local_via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);

        let mut dialog_guard = self.dialog.lock().await;
        if let Some(ref mut dlg) = *dialog_guard {
            let bye_req = dlg.create_bye(&local_via);
            let _ = self
                .transport
                .send_to(bye_req.serialize().as_bytes(), self.remote_addr)
                .await;
        }
        *dialog_guard = None;

        if let Some(media) = self.media_session.lock().await.take() {
            let _ = media.stop().await;
        }

        *st = CallState::Terminated {
            reason: DisconnectReason::LocalHangup,
        };
        let _ = self
            .event_tx
            .send(CallEvent::StateChanged(st.clone()))
            .await;

        info!(call_id = %self.call_id, "Incoming call hung up locally");
        Ok(())
    }

    /// Places active call on hold via in-dialog re-INVITE (RFC 3264 a=sendonly)
    pub async fn hold(&self) -> Result<(), String> {
        {
            let st = self.state.lock().await;
            if !matches!(*st, CallState::Active { .. }) {
                return Err("Cannot hold call that is not Active".into());
            }
        }

        let mut dialog_guard = self.dialog.lock().await;
        let dialog = dialog_guard
            .as_mut()
            .ok_or_else(|| "No active dialog for hold".to_string())?;

        let local_ip = self.transport.local_ip_for(self.remote_addr);
        let local_port = self.transport.local_addr().port();
        let local_via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);
        let contact = format!("<sip:plasma-sip@{}:{}>", local_ip, local_port);

        let bound_rtp_port = self
            .media_session
            .lock()
            .await
            .as_ref()
            .map(|m| m.local_addr().port())
            .unwrap_or(16000);

        let sdp = SdpSession::new(local_ip, bound_rtp_port, vec![8, 0, 101])
            .with_direction(sip_protocol::MediaDirection::SendOnly);
        let sdp_body = sdp.serialize();

        let reinvite = dialog.create_reinvite(&local_via, &contact, &sdp_body);
        let branch = reinvite
            .headers
            .get("Via")
            .and_then(|v| sip_protocol::extract_branch(v))
            .unwrap_or_else(sip_protocol::generate_branch);
        let key = sip_protocol::TransactionKey {
            branch,
            method: "INVITE".into(),
        };

        let mut rx = self.transactions.register_client_transaction(key).await;
        let serialized = reinvite.serialize();
        self.transport
            .send_to(serialized.as_bytes(), self.remote_addr)
            .await
            .map_err(|e| format!("Transport error on re-INVITE: {}", e))?;
        drop(dialog_guard);

        let _resp = match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await
        {
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
                .send_to(ack.serialize().as_bytes(), self.remote_addr)
                .await;
        }

        let mut st = self.state.lock().await;
        *st = CallState::Held;
        let _ = self
            .event_tx
            .send(CallEvent::StateChanged(CallState::Held))
            .await;

        info!(call_id = %self.call_id, "Incoming call successfully placed on hold");
        Ok(())
    }

    /// Resumes active call from hold via in-dialog re-INVITE (RFC 3264 a=sendrecv)
    pub async fn resume(&self) -> Result<(), String> {
        {
            let st = self.state.lock().await;
            if !matches!(*st, CallState::Held) {
                return Err("Cannot resume call that is not Held".into());
            }
        }

        let mut dialog_guard = self.dialog.lock().await;
        let dialog = dialog_guard
            .as_mut()
            .ok_or_else(|| "No active dialog for resume".to_string())?;

        let local_ip = self.transport.local_ip_for(self.remote_addr);
        let local_port = self.transport.local_addr().port();
        let local_via = format!("SIP/2.0/UDP {}:{}", local_ip, local_port);
        let contact = format!("<sip:plasma-sip@{}:{}>", local_ip, local_port);

        let bound_rtp_port = self
            .media_session
            .lock()
            .await
            .as_ref()
            .map(|m| m.local_addr().port())
            .unwrap_or(16000);

        let sdp = SdpSession::new(local_ip, bound_rtp_port, vec![8, 0, 101])
            .with_direction(sip_protocol::MediaDirection::SendRecv);
        let sdp_body = sdp.serialize();

        let reinvite = dialog.create_reinvite(&local_via, &contact, &sdp_body);
        let branch = reinvite
            .headers
            .get("Via")
            .and_then(|v| sip_protocol::extract_branch(v))
            .unwrap_or_else(sip_protocol::generate_branch);
        let key = sip_protocol::TransactionKey {
            branch,
            method: "INVITE".into(),
        };

        let mut rx = self.transactions.register_client_transaction(key).await;
        let serialized = reinvite.serialize();
        self.transport
            .send_to(serialized.as_bytes(), self.remote_addr)
            .await
            .map_err(|e| format!("Transport error on re-INVITE resume: {}", e))?;
        drop(dialog_guard);

        let _resp = match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await
        {
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
                .send_to(ack.serialize().as_bytes(), self.remote_addr)
                .await;
        }

        let new_state = CallState::Active {
            started_at: chrono::Utc::now(),
        };
        let mut st = self.state.lock().await;
        *st = new_state.clone();
        let _ = self.event_tx.send(CallEvent::StateChanged(new_state)).await;

        info!(call_id = %self.call_id, "Incoming call successfully resumed from hold");
        Ok(())
    }

    /// Handles an incoming in-dialog re-INVITE request (RFC 3261 Section 14 / RFC 3264)
    pub async fn handle_reinvite(&self, req: &SipRequest, src: SocketAddr) -> bool {
        let mut dialog_guard = self.dialog.lock().await;
        let dialog = match dialog_guard.as_mut() {
            Some(d) if d.matches_incoming_request(req) => d,
            _ => return false,
        };

        info!(call_id = %self.call_id, "Handling in-dialog re-INVITE in incoming session dialog");

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
                16000
            }
        };

        let local_ip = self.transport.local_ip_for(src);
        let answer_sdp = SdpSession::new(local_ip, bound_rtp_port, vec![8, 0, 101])
            .with_direction(answer_direction);
        let sdp_body = answer_sdp.serialize();

        let contact = format!(
            "<sip:plasma-sip@{}:{}>",
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
            let mut st = self.state.lock().await;
            if !matches!(*st, CallState::Held | CallState::Terminated { .. }) {
                *st = CallState::Held;
                let _ = self
                    .event_tx
                    .send(CallEvent::StateChanged(CallState::Held))
                    .await;
            }
        } else {
            let mut st = self.state.lock().await;
            if matches!(*st, CallState::Held) {
                let new_state = CallState::Active {
                    started_at: chrono::Utc::now(),
                };
                *st = new_state.clone();
                let _ = self.event_tx.send(CallEvent::StateChanged(new_state)).await;
            }
        }

        true
    }
}
