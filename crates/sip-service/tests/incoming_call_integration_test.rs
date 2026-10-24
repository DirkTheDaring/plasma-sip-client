//! Remediation R4 Integration Tests: Genuine Incoming SIP Calls and Desktop Events.
//!
//! Validates:
//! 1. SIP Transport listener receiving inbound INVITE requests.
//! 2. Provisional responses (100 Trying, 180 Ringing with To-tag).
//! 3. Account matching and call state tracking (RingingLocal).
//! 4. D-Bus signal emission (incoming_call, call_state_changed).
//! 5. Desktop notification and notification action handling (Answer/Reject).
//! 6. 200 OK with SDP Answer negotiation and in-dialog ACK absorption.
//! 7. Bidirectional RTP media stream establishment and statistics tracking.
//! 8. Rejection (603 Decline) and Busy line behavior (486 Busy Here).
//! 9. Remote CANCEL before answer (200 OK to CANCEL + 487 Request Terminated to INVITE).
//! 10. Remote and local BYE termination and SQLite call history persistence.
//! 11. Headless daemon operation (handling incoming calls without GUI).
//! 12. Repeated incoming call cycles with zero task or socket leaks.

use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

use sip_audio::{linear_to_alaw, AudioBackendMode, AudioEngine};
use sip_domain::{
    AccountId, CallDirection, CallRejectMode, CallState, DisconnectReason, SipAccount,
    TransportProtocol,
};
use sip_engine::SipClient;
use sip_media::rtp::RtpPacket;
use sip_platform::{NotificationAction, PlatformNotifier};
use sip_protocol::{
    generate_branch, generate_tag, SdpSession, SipMessage, SipMethod, SipRequest,
    TransactionManager,
};
use sip_service::CallManager;
use sip_storage::StorageManager;
use sip_transport::SipUdpTransport;

/// Helper to set up the client daemon environment:
/// In-memory SQLite storage, UDP transport, TransactionManager, CallManager, and SipClient receive loop.
async fn setup_daemon_environment() -> (
    Arc<CallManager>,
    Arc<StorageManager>,
    Arc<SipUdpTransport>,
    AccountId,
    tokio::task::JoinHandle<()>,
) {
    let storage = Arc::new(StorageManager::new_in_memory().expect("in-memory db"));
    assert!(storage.is_isolated_backend(), "Must use isolated credential backend");
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let mut call_manager =
        CallManager::new(storage.clone(), transport.clone(), transactions.clone());

    // Use synthetic audio engine to guarantee deterministic headless CI execution
    let audio_engine = AudioEngine::new_synthetic();
    assert_eq!(audio_engine.mode(), AudioBackendMode::Synthetic);
    call_manager = call_manager.with_audio_engine(audio_engine);
    let call_manager = Arc::new(call_manager);

    // Register CallManager as the incoming SIP request handler
    let sip_client = Arc::new(
        SipClient::new(transport.clone(), transactions.clone())
            .with_request_handler(call_manager.clone()),
    );
    let recv_loop = sip_client.start_receive_loop();

    // Seed account
    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "Daemon Account".into(),
        username: "620".into(),
        display_name: Some("Plasma Daemon".into()),
        domain: "fritz.box".into(),
        registrar: "127.0.0.1".into(),
        registrar_port: 5060,
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };
    storage.save_account(&account).expect("save account");
    storage.save_account_secret(account_id, "daemon-secret").expect("save secret");

    (call_manager, storage, transport, account_id, recv_loop)
}

#[tokio::test]
async fn test_r4_incoming_call_signaling_and_provisional_responses() {
    let (call_manager, _storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();

    // Create mock remote caller (PBX or external SIP phone)
    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_addr = caller_socket.local_addr().unwrap();

    let mut state_rx = call_manager.subscribe_call_events();
    let mut incoming_rx = call_manager.subscribe_incoming_calls();

    // 1. Remote caller sends INVITE with SDP offer
    let sdp_offer = SdpSession::new(caller_addr.ip(), 5006, vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
    );
    headers.insert(
        "From".into(),
        format!("\"Alice\" <sip:alice@fritz.box>;tag={}", from_tag),
    );
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:alice@{}>", caller_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket
        .send_to(invite.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 2. Caller receives 100 Trying and 180 Ringing
    let mut buf = vec![0u8; 4096];
    let mut received_180 = false;
    let mut to_tag_received = None;

    for _ in 0..2 {
        let (len, _) = tokio::time::timeout(
            Duration::from_millis(500),
            caller_socket.recv_from(&mut buf),
        )
        .await
        .expect("receive response timeout")
        .unwrap();
        let text = std::str::from_utf8(&buf[..len]).unwrap();
        if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
            if res.status_code == 180 {
                received_180 = true;
                assert_eq!(res.reason_phrase, "Ringing");
                assert_eq!(res.call_id(), Some(sip_call_id.as_str()));
                if let Some(to) = res.headers.get("To") {
                    if let Some((_, tag)) = to.split_once(";tag=") {
                        to_tag_received = Some(tag.to_string());
                    }
                }
            }
        }
    }

    assert!(received_180, "Expected 180 Ringing response from daemon");
    assert!(
        to_tag_received.is_some(),
        "180 Ringing MUST contain a local To-tag (RFC 3261 Section 13.3.1.4)"
    );

    // 3. Verify D-Bus signals and call state in CallManager
    let (inc_call_id, caller_identity) =
        tokio::time::timeout(Duration::from_millis(300), incoming_rx.recv())
            .await
            .expect("incoming call event timeout")
            .unwrap();

    assert!(caller_identity.contains("Alice"));

    let (st_call_id, state) = tokio::time::timeout(Duration::from_millis(300), state_rx.recv())
        .await
        .expect("call state event timeout")
        .unwrap();

    assert_eq!(inc_call_id, st_call_id);
    assert_eq!(state, CallState::RingingLocal);

    // Verify CallManager active call query
    let active_call = call_manager
        .get_active_call()
        .await
        .expect("active call info");
    assert_eq!(active_call.id, inc_call_id);
    assert_eq!(active_call.direction, CallDirection::Inbound);
    assert_eq!(active_call.state, CallState::RingingLocal);

    // Clean up with reject
    call_manager.reject_call(inc_call_id).await.unwrap();
}

#[tokio::test]
async fn test_r4_incoming_call_answer_and_bidirectional_audio() {
    let (call_manager, storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();

    // Create remote caller endpoint with RTP socket
    let caller_sip_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_sip_addr = caller_sip_socket.local_addr().unwrap();

    let caller_rtp_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let caller_rtp_addr = caller_rtp_socket.local_addr().unwrap();

    let mut state_rx = call_manager.subscribe_call_events();
    let mut incoming_rx = call_manager.subscribe_incoming_calls();

    // 1. Send incoming INVITE offering PCMA (8) and DTMF (101)
    let sdp_offer = SdpSession::new(caller_rtp_addr.ip(), caller_rtp_addr.port(), vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-answer-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_sip_addr, branch),
    );
    headers.insert(
        "From".into(),
        format!("\"Bob Caller\" <sip:bob@fritz.box>;tag={}", from_tag),
    );
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:bob@{}>", caller_sip_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_sip_socket
        .send_to(invite.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 2. Wait for incoming call event
    let (call_id, caller_id) = incoming_rx.recv().await.unwrap();
    assert!(caller_id.contains("Bob Caller"));

    // Read 100 Trying & 180 Ringing from network
    let mut buf = vec![0u8; 4096];
    let mut to_tag = None;
    for _ in 0..2 {
        let (len, _) = caller_sip_socket.recv_from(&mut buf).await.unwrap();
        let text = std::str::from_utf8(&buf[..len]).unwrap();
        if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
            if res.status_code == 180 {
                if let Some(t) = res.headers.get("To") {
                    if let Some((_, tag)) = t.split_once(";tag=") {
                        to_tag = Some(tag.to_string());
                    }
                }
            }
        }
    }
    let to_tag = to_tag.expect("received to_tag");

    // 3. Daemon answers the call (via D-Bus AnswerCall or notification action)
    call_manager
        .answer_call(call_id)
        .await
        .expect("answer incoming call");

    // 4. Remote caller receives 200 OK with SDP answer
    let (len_ok, _) = tokio::time::timeout(
        Duration::from_millis(500),
        caller_sip_socket.recv_from(&mut buf),
    )
    .await
    .expect("receive 200 OK timeout")
    .unwrap();
    let text_ok = std::str::from_utf8(&buf[..len_ok]).unwrap();
    let ok_res = match SipMessage::parse(text_ok).expect("parse 200 OK") {
        SipMessage::Response(r) => r,
        _ => panic!("Expected 200 OK response"),
    };
    assert_eq!(ok_res.status_code, 200);
    assert_eq!(ok_res.reason_phrase, "OK");

    // Validate negotiated SDP answer
    let sdp_answer = SdpSession::parse(&ok_res.body).expect("parse sdp answer");
    assert!(
        sdp_answer.payload_types.contains(&8),
        "SDP answer must negotiate PCMA (payload 8)"
    );
    let daemon_rtp_addr = SocketAddr::new(sdp_answer.connection_ip, sdp_answer.audio_port);

    // 5. Remote caller sends in-dialog ACK
    let mut ack_headers = std::collections::HashMap::new();
    ack_headers.insert(
        "Via".into(),
        format!(
            "SIP/2.0/UDP {};branch={}",
            caller_sip_addr,
            generate_branch()
        ),
    );
    ack_headers.insert(
        "From".into(),
        format!("\"Bob Caller\" <sip:bob@fritz.box>;tag={}", from_tag),
    );
    ack_headers.insert("To".into(), format!("<sip:620@127.0.0.1>;tag={}", to_tag));
    ack_headers.insert("Call-ID".into(), sip_call_id.clone());
    ack_headers.insert("CSeq".into(), "1 ACK".into());

    let ack = SipRequest {
        method: SipMethod::Ack,
        ruri: format!("sip:plasma-sip@{}", client_addr),
        headers: ack_headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };

    caller_sip_socket
        .send_to(ack.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 6. Exchange bidirectional RTP audio
    // Caller sends 6 RTP packets to daemon
    let tone = AudioEngine::generate_call_progress_tone(425.0, 20, 8000);
    let alaw_bytes: Vec<u8> = tone.iter().map(|&s| linear_to_alaw(s)).collect();
    for i in 0..6 {
        let pkt = RtpPacket::new(
            8,
            1000 + i,
            16000 + (i as u32 * 160),
            0x11223344,
            Bytes::copy_from_slice(&alaw_bytes),
        );
        let _ = caller_rtp_socket
            .send_to(&pkt.serialize(), daemon_rtp_addr)
            .await;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Caller receives client RTP packets
    let mut rtp_buf = vec![0u8; 1500];
    let mut packets_from_daemon = 0;
    let rtp_timeout = tokio::time::sleep(Duration::from_millis(600));
    tokio::pin!(rtp_timeout);

    loop {
        tokio::select! {
            _ = &mut rtp_timeout => break,
            recv_res = caller_rtp_socket.recv_from(&mut rtp_buf) => {
                if let Ok((len, _)) = recv_res {
                    if let Ok(pkt) = RtpPacket::parse(&rtp_buf[..len]) {
                        if pkt.payload_type == 8 {
                            packets_from_daemon += 1;
                            if packets_from_daemon >= 3 {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    assert!(
        packets_from_daemon >= 3,
        "Remote caller should have received RTP audio packets from answered daemon call"
    );

    // 7. Verify media statistics during active call
    let stats = call_manager.get_media_stats(call_id).await;
    assert!(
        stats.is_some(),
        "Media stats should be available for active incoming call"
    );
    let s = stats.unwrap();
    assert_eq!(s.negotiated_codec, "Pcma");
    assert!(
        s.packets_sent > 0,
        "Daemon should have transmitted RTP audio packets"
    );

    // 8. Remote caller terminates call with in-dialog BYE
    let mut bye_headers = std::collections::HashMap::new();
    bye_headers.insert(
        "Via".into(),
        format!(
            "SIP/2.0/UDP {};branch={}",
            caller_sip_addr,
            generate_branch()
        ),
    );
    bye_headers.insert(
        "From".into(),
        format!("\"Bob Caller\" <sip:bob@fritz.box>;tag={}", from_tag),
    );
    bye_headers.insert("To".into(), format!("<sip:620@127.0.0.1>;tag={}", to_tag));
    bye_headers.insert("Call-ID".into(), sip_call_id.clone());
    bye_headers.insert("CSeq".into(), "2 BYE".into());

    let bye = SipRequest {
        method: SipMethod::Bye,
        ruri: format!("sip:plasma-sip@{}", client_addr),
        headers: bye_headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };

    caller_sip_socket
        .send_to(bye.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // Verify 200 OK received for BYE
    let (len_bye_ok, _) = caller_sip_socket.recv_from(&mut buf).await.unwrap();
    let text_bye_ok = std::str::from_utf8(&buf[..len_bye_ok]).unwrap();
    let res_bye = SipMessage::parse(text_bye_ok).unwrap();
    match res_bye {
        SipMessage::Response(r) => assert_eq!(r.status_code, 200),
        _ => panic!("Expected 200 OK to BYE"),
    }

    // 9. Verify state transition to Terminated
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(300), state_rx.recv()).await
        {
            if id == call_id && matches!(state, CallState::Terminated { .. }) {
                break;
            }
        }
    }

    // 10. Verify call history in SQLite database
    let history = storage.list_call_history(10).expect("call history");
    assert!(!history.is_empty(), "Call history must contain record");
    let entry = &history[0];
    assert_eq!(entry.id, call_id);
    assert_eq!(entry.direction, CallDirection::Inbound);
    assert!(entry.remote_party.contains("Bob Caller"));
    assert_eq!(entry.disconnect_reason, "Call ended by remote party");
}

#[tokio::test]
async fn test_r4_incoming_call_reject() {
    let (call_manager, storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();
    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_addr = caller_socket.local_addr().unwrap();

    let mut incoming_rx = call_manager.subscribe_incoming_calls();
    let mut state_rx = call_manager.subscribe_call_events();

    let sdp_offer = SdpSession::new(caller_addr.ip(), 5006, vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-reject-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
    );
    headers.insert(
        "From".into(),
        format!("\"Spam Caller\" <sip:spam@fritz.box>;tag={}", from_tag),
    );
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:spam@{}>", caller_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket
        .send_to(invite.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    let (call_id, _) = incoming_rx.recv().await.unwrap();

    // Drain provisional responses (100 Trying, 180 Ringing)
    let mut buf = vec![0u8; 4096];
    for _ in 0..2 {
        let _ = caller_socket.recv_from(&mut buf).await.unwrap();
    }

    // Reject the call
    call_manager
        .reject_call(call_id)
        .await
        .expect("reject incoming call");

    // Verify remote caller receives 603 Decline
    let mut received_603 = false;
    for _ in 0..2 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(
            Duration::from_millis(400),
            caller_socket.recv_from(&mut buf),
        )
        .await
        {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                if res.status_code == 603 {
                    received_603 = true;
                    assert_eq!(res.reason_phrase, "Decline");
                    assert_eq!(res.call_id(), Some(sip_call_id.as_str()));
                    break;
                }
            }
        }
    }
    assert!(received_603, "Caller should have received 603 Decline");

    // Wait for CallState::Terminated
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(300), state_rx.recv()).await
        {
            if id == call_id
                && matches!(
                    state,
                    CallState::Terminated {
                        reason: DisconnectReason::Rejected
                    }
                )
            {
                break;
            }
        }
    }

    // Verify SQLite call history record has 'Call rejected' reason
    let history = storage.list_call_history(10).unwrap();
    let entry = history
        .iter()
        .find(|e| e.id == call_id)
        .expect("entry in history");
    assert_eq!(entry.direction, CallDirection::Inbound);
    assert_eq!(entry.disconnect_reason, "Call rejected");
}

#[tokio::test]
async fn test_r4_incoming_call_cancel_before_answer() {
    let (call_manager, storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();
    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_addr = caller_socket.local_addr().unwrap();

    let mut incoming_rx = call_manager.subscribe_incoming_calls();
    let mut state_rx = call_manager.subscribe_call_events();

    let sdp_offer = SdpSession::new(caller_addr.ip(), 5006, vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-cancel-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
    );
    headers.insert(
        "From".into(),
        format!("\"Caller Cancel\" <sip:caller@fritz.box>;tag={}", from_tag),
    );
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:caller@{}>", caller_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket
        .send_to(invite.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    let (call_id, _) = incoming_rx.recv().await.unwrap();

    // Drain provisional responses (100 Trying, 180 Ringing)
    let mut buf = vec![0u8; 4096];
    for _ in 0..2 {
        let _ = caller_socket.recv_from(&mut buf).await.unwrap();
    }

    // Remote caller sends CANCEL (RFC 3261 Section 9.1: identical Request-URI, To, From, Call-ID, top Via)
    let mut cancel_headers = std::collections::HashMap::new();
    cancel_headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
    );
    cancel_headers.insert(
        "From".into(),
        format!("\"Caller Cancel\" <sip:caller@fritz.box>;tag={}", from_tag),
    );
    cancel_headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    cancel_headers.insert("Call-ID".into(), sip_call_id.clone());
    cancel_headers.insert("CSeq".into(), "1 CANCEL".into());

    let cancel = SipRequest {
        method: SipMethod::Cancel,
        ruri: format!("sip:620@{}", client_addr),
        headers: cancel_headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };

    caller_socket
        .send_to(cancel.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // Caller receives 200 OK to CANCEL and 487 Request Terminated to INVITE
    let mut received_200_to_cancel = false;
    let mut received_487_to_invite = false;

    for _ in 0..3 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(
            Duration::from_millis(500),
            caller_socket.recv_from(&mut buf),
        )
        .await
        {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                if res.status_code == 200 && res.cseq().unwrap_or("").contains("CANCEL") {
                    received_200_to_cancel = true;
                }
                if res.status_code == 487 && res.cseq().unwrap_or("").contains("INVITE") {
                    received_487_to_invite = true;
                }
            }
        }
    }

    assert!(received_200_to_cancel, "Expected 200 OK response to CANCEL");
    assert!(
        received_487_to_invite,
        "Expected 487 Request Terminated response to INVITE"
    );

    // Verify call transitioned to Terminated { reason: Cancelled }
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(300), state_rx.recv()).await
        {
            if id == call_id
                && matches!(
                    state,
                    CallState::Terminated {
                        reason: DisconnectReason::Cancelled
                    }
                )
            {
                break;
            }
        }
    }

    // Verify SQLite call history record has 'Call cancelled' reason
    let history = storage.list_call_history(10).unwrap();
    let entry = history
        .iter()
        .find(|e| e.id == call_id)
        .expect("entry in history");
    assert_eq!(entry.disconnect_reason, "Call cancelled");
}

#[tokio::test]
async fn test_r4_incoming_call_busy_behavior() {
    let (call_manager, _storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();

    // 1. Establish first incoming call (Call 1) in RingingLocal state
    let caller1_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller1_addr = caller1_socket.local_addr().unwrap();

    let sdp_offer = SdpSession::new(caller1_addr.ip(), 5006, vec![8, 101]);
    let branch1 = generate_branch();
    let from_tag1 = generate_tag();
    let call_id1 = format!("call-1-{}@127.0.0.1", generate_tag());

    let mut headers1 = std::collections::HashMap::new();
    headers1.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller1_addr, branch1),
    );
    headers1.insert(
        "From".into(),
        format!("\"Caller 1\" <sip:caller1@fritz.box>;tag={}", from_tag1),
    );
    headers1.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers1.insert("Call-ID".into(), call_id1.clone());
    headers1.insert("CSeq".into(), "1 INVITE".into());
    headers1.insert("Contact".into(), format!("<sip:caller1@{}>", caller1_addr));
    headers1.insert("Content-Type".into(), "application/sdp".into());

    let invite1 = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers: headers1,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller1_socket
        .send_to(invite1.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    let mut buf = vec![0u8; 4096];
    let _ = caller1_socket.recv_from(&mut buf).await.unwrap(); // Receive 100/180

    // 2. Second remote caller places a call while Call 1 is ringing (busy line!)
    let caller2_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller2_addr = caller2_socket.local_addr().unwrap();

    let branch2 = generate_branch();
    let from_tag2 = generate_tag();
    let call_id2 = format!("call-2-{}@127.0.0.1", generate_tag());

    let mut headers2 = std::collections::HashMap::new();
    headers2.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller2_addr, branch2),
    );
    headers2.insert(
        "From".into(),
        format!("\"Caller 2\" <sip:caller2@fritz.box>;tag={}", from_tag2),
    );
    headers2.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers2.insert("Call-ID".into(), call_id2.clone());
    headers2.insert("CSeq".into(), "1 INVITE".into());
    headers2.insert("Contact".into(), format!("<sip:caller2@{}>", caller2_addr));
    headers2.insert("Content-Type".into(), "application/sdp".into());

    let invite2 = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers: headers2,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller2_socket
        .send_to(invite2.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 3. Caller 2 MUST immediately receive 486 Busy Here
    let (len2, _) = tokio::time::timeout(
        Duration::from_millis(500),
        caller2_socket.recv_from(&mut buf),
    )
    .await
    .expect("receive busy timeout")
    .unwrap();
    let text2 = std::str::from_utf8(&buf[..len2]).unwrap();
    let res2 = match SipMessage::parse(text2).expect("parse response") {
        SipMessage::Response(r) => r,
        _ => panic!("Expected response"),
    };
    assert_eq!(res2.status_code, 486);
    assert_eq!(res2.reason_phrase, "Busy Here");
    assert_eq!(res2.call_id(), Some(call_id2.as_str()));

    // 4. Verify Call 1 is still undisturbed in RingingLocal state
    let active = call_manager.get_active_call().await.unwrap();
    assert_eq!(active.direction, CallDirection::Inbound);
    assert_eq!(active.state, CallState::RingingLocal);

    // Clean up Call 1
    call_manager.reject_call(active.id).await.unwrap();
}

#[tokio::test]
async fn test_r4_notification_action_triggers_answer() {
    let (call_manager, _storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();
    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_addr = caller_socket.local_addr().unwrap();

    // Configure PlatformNotifier
    let notifier = PlatformNotifier::with_connection(None); // Headless mode
    call_manager.set_notifier(notifier.clone()).await;

    let mut incoming_rx = call_manager.subscribe_incoming_calls();
    let mut state_rx = call_manager.subscribe_call_events();

    // Listen for notification actions and trigger CallManager methods
    let mut action_rx = notifier.subscribe_actions();
    let cm_clone = call_manager.clone();
    tokio::spawn(async move {
        while let Ok((call_id, action)) = action_rx.recv().await {
            match action {
                NotificationAction::Answer => {
                    let _ = cm_clone.answer_call(call_id).await;
                }
                NotificationAction::Reject => {
                    let _ = cm_clone.reject_call(call_id).await;
                }
                NotificationAction::Open => {}
            }
        }
    });

    let sdp_offer = SdpSession::new(caller_addr.ip(), 5006, vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-notif-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
    );
    headers.insert(
        "From".into(),
        format!("\"Notifier Caller\" <sip:notif@fritz.box>;tag={}", from_tag),
    );
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:notif@{}>", caller_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket
        .send_to(invite.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    let (call_id, _) = incoming_rx.recv().await.unwrap();

    // Drain provisional responses (100 Trying, 180 Ringing)
    let mut buf = vec![0u8; 4096];
    for _ in 0..2 {
        let _ = caller_socket.recv_from(&mut buf).await.unwrap();
    }

    // User clicks Answer button on the desktop notification
    notifier.trigger_action(call_id, NotificationAction::Answer);

    // Verify 200 OK received
    let mut received_200 = false;
    for _ in 0..3 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(
            Duration::from_millis(600),
            caller_socket.recv_from(&mut buf),
        )
        .await
        {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                if res.status_code == 200 {
                    received_200 = true;
                    assert_eq!(res.reason_phrase, "OK");
                    break;
                }
            }
        }
    }
    assert!(
        received_200,
        "Expected 200 OK after answering via notification action"
    );

    // Wait for CallState::Active
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(300), state_rx.recv()).await
        {
            if id == call_id && matches!(state, CallState::Active { .. }) {
                break;
            }
        }
    }

    call_manager.hangup_call(call_id).await.unwrap();
}

#[tokio::test]
async fn test_r4_repeated_incoming_calls_cleanup_no_leaks() {
    let (call_manager, storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    let client_addr = client_transport.local_addr();

    for i in 0..3 {
        let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let caller_addr = caller_socket.local_addr().unwrap();

        let mut incoming_rx = call_manager.subscribe_incoming_calls();
        let mut state_rx = call_manager.subscribe_call_events();

        let sdp_offer = SdpSession::new(caller_addr.ip(), 5006, vec![8, 101]);
        let branch = generate_branch();
        let from_tag = generate_tag();
        let sip_call_id = format!("call-leak-{}-{}@127.0.0.1", i, generate_tag());

        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Via".into(),
            format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
        );
        headers.insert(
            "From".into(),
            format!("\"Caller {}\" <sip:c{}@fritz.box>;tag={}", i, i, from_tag),
        );
        headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
        headers.insert("Call-ID".into(), sip_call_id.clone());
        headers.insert("CSeq".into(), "1 INVITE".into());
        headers.insert("Contact".into(), format!("<sip:c{}@{}>", i, caller_addr));
        headers.insert("Content-Type".into(), "application/sdp".into());

        let invite = SipRequest {
            method: SipMethod::Invite,
            ruri: format!("sip:620@{}", client_addr),
            headers,
            raw_headers: Vec::new(),
            body: sdp_offer.serialize(),
        };

        caller_socket
            .send_to(invite.serialize().as_bytes(), client_addr)
            .await
            .unwrap();

        let (call_id, _) = incoming_rx.recv().await.unwrap();

        // Drain provisional responses (100 Trying, 180 Ringing)
        let mut buf = vec![0u8; 4096];
        for _ in 0..2 {
            let _ = caller_socket.recv_from(&mut buf).await.unwrap();
        }

        // Answer call
        call_manager.answer_call(call_id).await.unwrap();

        // Receive 200 OK
        let mut received_200 = false;
        for _ in 0..2 {
            if let Ok(Ok((len, _))) = tokio::time::timeout(
                Duration::from_millis(400),
                caller_socket.recv_from(&mut buf),
            )
            .await
            {
                let text = std::str::from_utf8(&buf[..len]).unwrap();
                if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                    if res.status_code == 200 {
                        received_200 = true;
                        break;
                    }
                }
            }
        }
        assert!(received_200, "Caller should have received 200 OK");

        // Hangup call
        call_manager.hangup_call(call_id).await.unwrap();

        // Wait for Terminated event
        loop {
            if let Ok(Ok((id, state))) =
                tokio::time::timeout(Duration::from_millis(300), state_rx.recv()).await
            {
                if id == call_id && matches!(state, CallState::Terminated { .. }) {
                    break;
                }
            }
        }

        // Verify clean memory state: no active call or session remaining
        assert!(call_manager.get_active_call().await.is_none());
        assert!(call_manager.get_media_stats(call_id).await.is_none());
    }

    // Verify all 3 calls were logged
    let history = storage.list_call_history(10).unwrap();
    assert_eq!(history.len(), 3, "Expected 3 logged calls in history");
}

#[tokio::test]
async fn test_r4_untrusted_remote_origin_invite_rejected() {
    let (call_manager, _storage, _client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;

    // Simulate an unsolicited INVITE from an external internet scanner IP
    let untrusted_src: std::net::SocketAddr = "198.51.100.23:5060".parse().unwrap();
    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        "SIP/2.0/UDP 198.51.100.23:5060;branch=z9hG4bK-scanner".into(),
    );
    headers.insert(
        "From".into(),
        "\"friendly-scanner\" <sip:100@198.51.100.23>;tag=scanner-tag".into(),
    );
    headers.insert("To".into(), "<sip:620@target>".into());
    headers.insert("Call-ID".into(), "scanner-call-id-999@198.51.100.23".into());
    headers.insert("CSeq".into(), "1 INVITE".into());

    let req = sip_protocol::SipRequest {
        method: sip_protocol::SipMethod::Invite,
        ruri: "sip:620@target".into(),
        headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };

    // Handler must reject and not activate any call session
    let handled = call_manager
        .handle_incoming_invite(req, untrusted_src)
        .await;
    assert!(
        handled,
        "Handled should return true (403 Forbidden dispatched)"
    );
    assert!(
        call_manager.get_active_call().await.is_none(),
        "No active call should be created for untrusted origin"
    );
}

#[tokio::test]
async fn test_r4_in_dialog_reinvite_hold_and_resume() {
    let (call_manager, _storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;
    let client_addr = client_transport.local_addr();

    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_addr = caller_socket.local_addr().unwrap();

    let mut state_rx = call_manager.subscribe_call_events();

    // 1. Initial INVITE
    let sdp_offer = SdpSession::new(caller_addr.ip(), 5008, vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-reinv-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, branch),
    );
    headers.insert(
        "From".into(),
        format!("\"Alice\" <sip:alice@fritz.box>;tag={}", from_tag),
    );
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:alice@{}>", caller_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket
        .send_to(invite.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 2. Consume 100 Trying & 180 Ringing, answer call
    let mut buf = vec![0u8; 4096];
    let mut to_tag = String::new();

    for _ in 0..2 {
        let (len, _) = tokio::time::timeout(
            Duration::from_millis(500),
            caller_socket.recv_from(&mut buf),
        )
        .await
        .expect("timeout receiving provisional")
        .unwrap();
        let text = std::str::from_utf8(&buf[..len]).unwrap();
        if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
            if res.status_code == 180 {
                if let Some(t) = res.headers.get("To") {
                    if let Some((_, tag)) = t.split_once(";tag=") {
                        to_tag = tag.to_string();
                    }
                }
            }
        }
    }
    assert!(!to_tag.is_empty(), "Must receive To tag from 180 Ringing");

    let call_id = call_manager.get_active_call().await.unwrap().id;
    call_manager.answer_call(call_id).await.unwrap();

    // 3. Receive 200 OK to INVITE and send ACK
    let (len, _) = tokio::time::timeout(
        Duration::from_millis(500),
        caller_socket.recv_from(&mut buf),
    )
    .await
    .expect("timeout receiving 200 OK")
    .unwrap();
    let text = std::str::from_utf8(&buf[..len]).unwrap();
    let resp = match SipMessage::parse(text).unwrap() {
        SipMessage::Response(r) => r,
        _ => panic!("Expected 200 OK response"),
    };
    assert_eq!(resp.status_code, 200);

    let mut ack_headers = std::collections::HashMap::new();
    ack_headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, generate_branch()),
    );
    ack_headers.insert(
        "From".into(),
        format!("\"Alice\" <sip:alice@fritz.box>;tag={}", from_tag),
    );
    ack_headers.insert("To".into(), format!("<sip:620@127.0.0.1>;tag={}", to_tag));
    ack_headers.insert("Call-ID".into(), sip_call_id.clone());
    ack_headers.insert("CSeq".into(), "1 ACK".into());
    let ack = SipRequest {
        method: SipMethod::Ack,
        ruri: format!("sip:plasma-sip@{}", client_addr),
        headers: ack_headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };
    caller_socket
        .send_to(ack.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // Confirm state became Active
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(500), state_rx.recv()).await
        {
            if id == call_id && matches!(state, CallState::Active { .. }) {
                break;
            }
        }
    }

    // 4. Remote PBX sends in-dialog re-INVITE with a=sendonly (Remote puts local client on hold)
    let sdp_hold = SdpSession::new(caller_addr.ip(), 5008, vec![8, 101])
        .with_direction(sip_protocol::MediaDirection::SendOnly);

    let mut reinv_headers = std::collections::HashMap::new();
    reinv_headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, generate_branch()),
    );
    reinv_headers.insert(
        "From".into(),
        format!("\"Alice\" <sip:alice@fritz.box>;tag={}", from_tag),
    );
    reinv_headers.insert("To".into(), format!("<sip:620@127.0.0.1>;tag={}", to_tag));
    reinv_headers.insert("Call-ID".into(), sip_call_id.clone());
    reinv_headers.insert("CSeq".into(), "2 INVITE".into());
    reinv_headers.insert("Contact".into(), format!("<sip:alice@{}>", caller_addr));
    reinv_headers.insert("Content-Type".into(), "application/sdp".into());

    let reinv = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:plasma-sip@{}", client_addr),
        headers: reinv_headers,
        raw_headers: Vec::new(),
        body: sdp_hold.serialize(),
    };

    caller_socket
        .send_to(reinv.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 5. Client must respond 200 OK with SDP Answer containing a=recvonly (RFC 3264 §6.1)
    let (len, _) = tokio::time::timeout(
        Duration::from_millis(500),
        caller_socket.recv_from(&mut buf),
    )
    .await
    .expect("timeout receiving 200 OK to re-INVITE")
    .unwrap();
    let text = std::str::from_utf8(&buf[..len]).unwrap();
    let reinv_resp = match SipMessage::parse(text).unwrap() {
        SipMessage::Response(r) => r,
        _ => panic!("Expected 200 OK to re-INVITE"),
    };
    assert_eq!(reinv_resp.status_code, 200);
    assert!(
        reinv_resp.body.contains("a=recvonly"),
        "Answer to sendonly re-INVITE must be recvonly (RFC 3264 §6.1): {}",
        reinv_resp.body
    );

    // 6. Verify CallState transitioned to Held
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(500), state_rx.recv()).await
        {
            if id == call_id && matches!(state, CallState::Held) {
                break;
            }
        }
    }
    assert_eq!(
        call_manager.get_active_call().await.unwrap().state,
        CallState::Held
    );

    // Caller sends ACK for re-INVITE
    let mut ack2_headers = std::collections::HashMap::new();
    ack2_headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, generate_branch()),
    );
    ack2_headers.insert(
        "From".into(),
        format!("\"Alice\" <sip:alice@fritz.box>;tag={}", from_tag),
    );
    ack2_headers.insert("To".into(), format!("<sip:620@127.0.0.1>;tag={}", to_tag));
    ack2_headers.insert("Call-ID".into(), sip_call_id.clone());
    ack2_headers.insert("CSeq".into(), "2 ACK".into());
    let ack2 = SipRequest {
        method: SipMethod::Ack,
        ruri: format!("sip:plasma-sip@{}", client_addr),
        headers: ack2_headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };
    caller_socket
        .send_to(ack2.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 7. Remote PBX resumes call with in-dialog re-INVITE containing a=sendrecv
    let sdp_resume = SdpSession::new(caller_addr.ip(), 5008, vec![8, 101])
        .with_direction(sip_protocol::MediaDirection::SendRecv);

    let mut resume_headers = std::collections::HashMap::new();
    resume_headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", caller_addr, generate_branch()),
    );
    resume_headers.insert(
        "From".into(),
        format!("\"Alice\" <sip:alice@fritz.box>;tag={}", from_tag),
    );
    resume_headers.insert("To".into(), format!("<sip:620@127.0.0.1>;tag={}", to_tag));
    resume_headers.insert("Call-ID".into(), sip_call_id.clone());
    resume_headers.insert("CSeq".into(), "3 INVITE".into());
    resume_headers.insert("Contact".into(), format!("<sip:alice@{}>", caller_addr));
    resume_headers.insert("Content-Type".into(), "application/sdp".into());

    let resume_inv = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:plasma-sip@{}", client_addr),
        headers: resume_headers,
        raw_headers: Vec::new(),
        body: sdp_resume.serialize(),
    };

    caller_socket
        .send_to(resume_inv.serialize().as_bytes(), client_addr)
        .await
        .unwrap();

    // 8. Client must respond 200 OK with SDP Answer containing a=sendrecv
    let (len, _) = tokio::time::timeout(
        Duration::from_millis(500),
        caller_socket.recv_from(&mut buf),
    )
    .await
    .expect("timeout receiving 200 OK to resume re-INVITE")
    .unwrap();
    let text = std::str::from_utf8(&buf[..len]).unwrap();
    let resume_resp = match SipMessage::parse(text).unwrap() {
        SipMessage::Response(r) => r,
        _ => panic!("Expected 200 OK to resume re-INVITE"),
    };
    assert_eq!(resume_resp.status_code, 200);
    assert!(
        resume_resp.body.contains("a=sendrecv"),
        "Answer to sendrecv re-INVITE must be sendrecv: {}",
        resume_resp.body
    );

    // 9. Verify CallState transitioned back to Active
    loop {
        if let Ok(Ok((id, state))) =
            tokio::time::timeout(Duration::from_millis(500), state_rx.recv()).await
        {
            if id == call_id && matches!(state, CallState::Active { .. }) {
                break;
            }
        }
    }
    assert!(matches!(
        call_manager.get_active_call().await.unwrap().state,
        CallState::Active { .. }
    ));

    // Clean up with BYE
    call_manager.hangup_call(call_id).await.unwrap();
}

#[tokio::test]
async fn test_r4_incoming_call_configurable_reject_modes() {
    let (call_manager, storage, client_transport, _account_id, _recv_loop) =
        setup_daemon_environment().await;
    let client_addr = client_transport.local_addr();

    // 1. Test 486 Busy Here reject mode
    storage.save_reject_mode(CallRejectMode::BusyHere486).unwrap();
    assert_eq!(storage.get_reject_mode().unwrap(), CallRejectMode::BusyHere486);

    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let caller_addr = caller_socket.local_addr().unwrap();
    let mut incoming_rx = call_manager.subscribe_incoming_calls();

    let sdp_offer = SdpSession::new(caller_addr.ip(), 5008, vec![8, 101]);
    let branch = generate_branch();
    let from_tag = generate_tag();
    let sip_call_id = format!("call-reject-486-{}@127.0.0.1", generate_tag());

    let mut headers = std::collections::HashMap::new();
    headers.insert("Via".into(), format!("SIP/2.0/UDP {};branch={}", caller_addr, branch));
    headers.insert("From".into(), format!("\"Caller\" <sip:caller@fritz.box>;tag={}", from_tag));
    headers.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers.insert("Call-ID".into(), sip_call_id.clone());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:caller@{}>", caller_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let invite = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket.send_to(invite.serialize().as_bytes(), client_addr).await.unwrap();
    let (call_id, _) = incoming_rx.recv().await.unwrap();

    // Drain provisional responses
    let mut buf = vec![0u8; 4096];
    let _ = caller_socket.recv_from(&mut buf).await.unwrap();
    let _ = caller_socket.recv_from(&mut buf).await.unwrap();

    // Reject with configured 486 mode
    call_manager.reject_call(call_id).await.unwrap();

    let mut received_486 = false;
    for _ in 0..2 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(Duration::from_millis(400), caller_socket.recv_from(&mut buf)).await {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                if res.status_code == 486 {
                    received_486 = true;
                    assert_eq!(res.reason_phrase, "Busy Here");
                    break;
                }
            }
        }
    }
    assert!(received_486, "Caller should have received 486 Busy Here");

    // 2. Test 600 Busy Everywhere reject mode
    storage.save_reject_mode(CallRejectMode::BusyEverywhere600).unwrap();
    let sip_call_id2 = format!("call-reject-600-{}@127.0.0.1", generate_tag());
    let mut headers2 = std::collections::HashMap::new();
    headers2.insert("Via".into(), format!("SIP/2.0/UDP {};branch={}", caller_addr, generate_branch()));
    headers2.insert("From".into(), format!("\"Caller\" <sip:caller@fritz.box>;tag={}", generate_tag()));
    headers2.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers2.insert("Call-ID".into(), sip_call_id2.clone());
    headers2.insert("CSeq".into(), "1 INVITE".into());
    headers2.insert("Contact".into(), format!("<sip:caller@{}>", caller_addr));
    headers2.insert("Content-Type".into(), "application/sdp".into());

    let invite2 = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers: headers2,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket.send_to(invite2.serialize().as_bytes(), client_addr).await.unwrap();
    let (call_id2, _) = incoming_rx.recv().await.unwrap();
    let _ = caller_socket.recv_from(&mut buf).await.unwrap();
    let _ = caller_socket.recv_from(&mut buf).await.unwrap();

    call_manager.reject_call(call_id2).await.unwrap();

    let mut received_600 = false;
    for _ in 0..2 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(Duration::from_millis(400), caller_socket.recv_from(&mut buf)).await {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                if res.status_code == 600 {
                    received_600 = true;
                    assert_eq!(res.reason_phrase, "Busy Everywhere");
                    break;
                }
            }
        }
    }
    assert!(received_600, "Caller should have received 600 Busy Everywhere");

    // 3. Test Answer & Drop reject mode
    storage.save_reject_mode(CallRejectMode::AnswerAndDrop).unwrap();
    let sip_call_id3 = format!("call-reject-drop-{}@127.0.0.1", generate_tag());
    let mut headers3 = std::collections::HashMap::new();
    headers3.insert("Via".into(), format!("SIP/2.0/UDP {};branch={}", caller_addr, generate_branch()));
    headers3.insert("From".into(), format!("\"Caller\" <sip:caller@fritz.box>;tag={}", generate_tag()));
    headers3.insert("To".into(), "<sip:620@127.0.0.1>".into());
    headers3.insert("Call-ID".into(), sip_call_id3.clone());
    headers3.insert("CSeq".into(), "1 INVITE".into());
    headers3.insert("Contact".into(), format!("<sip:caller@{}>", caller_addr));
    headers3.insert("Content-Type".into(), "application/sdp".into());

    let invite3 = SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:620@{}", client_addr),
        headers: headers3,
        raw_headers: Vec::new(),
        body: sdp_offer.serialize(),
    };

    caller_socket.send_to(invite3.serialize().as_bytes(), client_addr).await.unwrap();
    let (call_id3, _) = incoming_rx.recv().await.unwrap();
    let _ = caller_socket.recv_from(&mut buf).await.unwrap();
    let _ = caller_socket.recv_from(&mut buf).await.unwrap();

    call_manager.reject_call(call_id3).await.unwrap();

    let mut received_200 = false;
    for _ in 0..2 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(Duration::from_millis(400), caller_socket.recv_from(&mut buf)).await {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                if res.status_code == 200 {
                    received_200 = true;
                    break;
                }
            }
        }
    }
    assert!(received_200, "Caller should have received 200 OK from Answer & Drop");

    // Receive the immediate BYE
    let mut received_bye = false;
    for _ in 0..2 {
        if let Ok(Ok((len, _))) = tokio::time::timeout(Duration::from_millis(400), caller_socket.recv_from(&mut buf)).await {
            let text = std::str::from_utf8(&buf[..len]).unwrap();
            if let Ok(SipMessage::Request(req)) = SipMessage::parse(text) {
                if req.method == SipMethod::Bye {
                    received_bye = true;
                    break;
                }
            }
        }
    }
    assert!(received_bye, "Caller should have received immediate BYE from Answer & Drop");
}

