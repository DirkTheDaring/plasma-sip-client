//! Automated outgoing SIP call scenarios against a controlled SIP PBX endpoint.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use sip_domain::{AccountId, CallState, DisconnectReason, SipAccount, TransportProtocol};
use sip_engine::OutgoingCallSession;
use sip_protocol::{SipMessage, SipMethod, TransactionManager};
use sip_transport::SipUdpTransport;

/// Mock SIP PBX/UAS that responds to INVITE with customizable behavior
struct MockSipPbx {
    local_addr: SocketAddr,
}

enum PbxResponseBehavior {
    Answer200Ok,
    Busy486,
    Reject603,
    RingIndefinitely,
}

impl MockSipPbx {
    async fn start(behavior: PbxResponseBehavior) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind mock pbx");
        let local_addr = socket.local_addr().expect("local addr");
        let socket = Arc::new(socket);

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                let (len, client_addr) = match socket.recv_from(&mut buf).await {
                    Ok(r) => r,
                    Err(_) => break,
                };

                let text = match std::str::from_utf8(&buf[..len]) {
                    Ok(t) => t,
                    Err(_) => continue,
                };

                let msg = match SipMessage::parse(text) {
                    Ok(SipMessage::Request(req)) => req,
                    _ => continue,
                };

                let via = msg.headers.get("Via").cloned().unwrap_or_default();
                let from = msg.headers.get("From").cloned().unwrap_or_default();
                let to = msg.headers.get("To").cloned().unwrap_or_default();
                let call_id = msg.headers.get("Call-ID").cloned().unwrap_or_default();
                let cseq = msg.headers.get("CSeq").cloned().unwrap_or_default();

                match msg.method {
                    SipMethod::Invite => {
                        // 1. Always send 100 Trying
                        let trying = format!(
                            "SIP/2.0 100 Trying\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq
                        );
                        let _ = socket.send_to(trying.as_bytes(), client_addr).await;

                        match behavior {
                            PbxResponseBehavior::Answer200Ok => {
                                // 2. Send 180 Ringing
                                let ringing = format!(
                                    "SIP/2.0 180 Ringing\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-123\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                                    via, from, to, call_id, cseq
                                );
                                let _ = socket.send_to(ringing.as_bytes(), client_addr).await;

                                // 3. Send 200 OK with SDP answer
                                let sdp_answer = "v=0\r\no=mockpbx 100 1 IN IP4 127.0.0.1\r\ns=MockPBX\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 20000 RTP/AVP 8 0 101\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n";
                                let ok_200 = format!(
                                    "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-123\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:bob@127.0.0.1:{}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                                    via,
                                    from,
                                    to,
                                    call_id,
                                    cseq,
                                    local_addr.port(),
                                    sdp_answer.len(),
                                    sdp_answer
                                );
                                let _ = socket.send_to(ok_200.as_bytes(), client_addr).await;
                            }
                            PbxResponseBehavior::Busy486 => {
                                let busy = format!(
                                    "SIP/2.0 486 Busy Here\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-123\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                                    via, from, to, call_id, cseq
                                );
                                let _ = socket.send_to(busy.as_bytes(), client_addr).await;
                            }
                            PbxResponseBehavior::Reject603 => {
                                let decline = format!(
                                    "SIP/2.0 603 Decline\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-123\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                                    via, from, to, call_id, cseq
                                );
                                let _ = socket.send_to(decline.as_bytes(), client_addr).await;
                            }
                            PbxResponseBehavior::RingIndefinitely => {
                                let ringing = format!(
                                    "SIP/2.0 180 Ringing\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-123\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                                    via, from, to, call_id, cseq
                                );
                                let _ = socket.send_to(ringing.as_bytes(), client_addr).await;
                            }
                        }
                    }
                    SipMethod::Ack => {
                        // Received ACK for 200 OK
                    }
                    SipMethod::Cancel => {
                        // CANCEL received: respond 200 OK to CANCEL and 487 Request Terminated to INVITE
                        let cancel_ok = format!(
                            "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq
                        );
                        let _ = socket.send_to(cancel_ok.as_bytes(), client_addr).await;

                        let terminated = format!(
                            "SIP/2.0 487 Request Terminated\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-123\r\nCall-ID: {}\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id
                        );
                        let _ = socket.send_to(terminated.as_bytes(), client_addr).await;
                    }
                    SipMethod::Bye => {
                        let bye_ok = format!(
                            "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq
                        );
                        let _ = socket.send_to(bye_ok.as_bytes(), client_addr).await;
                    }
                    _ => {}
                }
            }
        });

        Self { local_addr }
    }
}

fn dummy_account(pbx_addr: SocketAddr) -> SipAccount {
    SipAccount {
        id: AccountId::new(),
        name: "Test Caller".into(),
        username: "620".into(),
        display_name: Some("Caller".into()),
        domain: "fritz.box".into(),
        registrar: pbx_addr.ip().to_string(),
        registrar_port: pbx_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    }
}

#[tokio::test]
async fn test_successful_outgoing_call_and_hangup() {
    let mock_pbx = MockSipPbx::start(PbxResponseBehavior::Answer200Ok).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let trans_clone = transactions.clone();
    let trans_net = transport.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = trans_net.recv_from(&mut buf).await {
                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                    if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                        trans_clone.handle_response(res).await;
                    }
                }
            }
        }
    });

    let (event_tx, _event_rx) = mpsc::channel(32);
    let session = OutgoingCallSession::new(
        dummy_account(mock_pbx.local_addr),
        "secret".into(),
        "sip:bob@127.0.0.1".into(),
        10000,
        transport,
        transactions,
        event_tx,
    );

    let start_res = session.start(mock_pbx.local_addr).await;
    assert!(start_res.is_ok(), "Outgoing call setup must succeed");

    let state = session.get_state().await;
    assert!(matches!(state, CallState::Active { .. }));

    // Now test local hangup
    let hangup_res = session.hangup(mock_pbx.local_addr).await;
    assert!(hangup_res.is_ok(), "Hangup must succeed");

    let final_state = session.get_state().await;
    assert_eq!(
        final_state,
        CallState::Terminated {
            reason: DisconnectReason::LocalHangup,
        }
    );
}

#[tokio::test]
async fn test_outgoing_call_busy() {
    let mock_pbx = MockSipPbx::start(PbxResponseBehavior::Busy486).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let trans_clone = transactions.clone();
    let trans_net = transport.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = trans_net.recv_from(&mut buf).await {
                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                    if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                        trans_clone.handle_response(res).await;
                    }
                }
            }
        }
    });

    let (event_tx, _event_rx) = mpsc::channel(32);
    let session = OutgoingCallSession::new(
        dummy_account(mock_pbx.local_addr),
        "secret".into(),
        "sip:bob@127.0.0.1".into(),
        10000,
        transport,
        transactions,
        event_tx,
    );

    let start_res = session.start(mock_pbx.local_addr).await;
    assert!(start_res.is_err(), "Busy call must return error");

    let final_state = session.get_state().await;
    assert_eq!(
        final_state,
        CallState::Terminated {
            reason: DisconnectReason::Busy,
        }
    );
}

#[tokio::test]
async fn test_outgoing_call_rejected() {
    let mock_pbx = MockSipPbx::start(PbxResponseBehavior::Reject603).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let trans_clone = transactions.clone();
    let trans_net = transport.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = trans_net.recv_from(&mut buf).await {
                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                    if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                        trans_clone.handle_response(res).await;
                    }
                }
            }
        }
    });

    let (event_tx, _event_rx) = mpsc::channel(32);
    let session = OutgoingCallSession::new(
        dummy_account(mock_pbx.local_addr),
        "secret".into(),
        "sip:bob@127.0.0.1".into(),
        10000,
        transport,
        transactions,
        event_tx,
    );

    let start_res = session.start(mock_pbx.local_addr).await;
    assert!(start_res.is_err(), "Rejected call must return error");

    let final_state = session.get_state().await;
    assert_eq!(
        final_state,
        CallState::Terminated {
            reason: DisconnectReason::Rejected,
        }
    );
}

#[tokio::test]
async fn test_outgoing_call_cancel() {
    let mock_pbx = MockSipPbx::start(PbxResponseBehavior::RingIndefinitely).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let trans_clone = transactions.clone();
    let trans_net = transport.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = trans_net.recv_from(&mut buf).await {
                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                    if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                        trans_clone.handle_response(res).await;
                    }
                }
            }
        }
    });

    let (event_tx, _event_rx) = mpsc::channel(32);
    let session = Arc::new(OutgoingCallSession::new(
        dummy_account(mock_pbx.local_addr),
        "secret".into(),
        "sip:bob@127.0.0.1".into(),
        10000,
        transport,
        transactions,
        event_tx,
    ));

    // Cancel while ringing
    let session_cancel = session.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = session_cancel.cancel(mock_pbx.local_addr).await;
    });

    let _ = session.start(mock_pbx.local_addr).await;

    let final_state = session.get_state().await;
    assert_eq!(
        final_state,
        CallState::Terminated {
            reason: DisconnectReason::Cancelled,
        }
    );
}
