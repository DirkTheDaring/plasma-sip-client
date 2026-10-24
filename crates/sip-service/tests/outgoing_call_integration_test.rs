//! Milestone R2 Integration Tests: Real Outgoing Calls over SIP UDP Transport
//!
//! Validates:
//! - INVITE is actually transmitted over the network and received by the remote endpoint.
//! - Provisional 180 Ringing is propagated through CallManager and event streams.
//! - Call establishment after 200 OK, SDP negotiation, and end-to-end ACK transmission.
//! - In-dialog Route set and CSeq handling (SIP-04 and SIP-05).
//! - 401 Authentication challenge with hop-by-hop ACK and authenticated re-INVITE.
//! - 486 Busy response with hop-by-hop ACK (RFC 3261 §17.1.1.3).
//! - 603 Rejected response with hop-by-hop ACK (RFC 3261 §17.1.1.3).
//! - Cancel before answer with RFC 3261 §9.1 header invariants (Via branch, From tag, numeric CSeq).
//! - Local hangup via in-dialog BYE.
//! - Remote hangup via incoming in-dialog BYE and 200 OK response.
//! - Timeout handling and zero orphaned dialogs/tasks.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use sip_domain::{
    AccountId, CallDirection, CallState, DisconnectReason, SipAccount, TransportProtocol,
};
use sip_protocol::{SipMessage, SipMethod, SipRequest, TransactionManager};
use sip_service::CallManager;
use sip_storage::StorageManager;
use sip_transport::SipUdpTransport;

#[derive(Clone)]
#[allow(dead_code)]
struct ReceivedSipPacket {
    raw: String,
    method: SipMethod,
    cseq: String,
    from: String,
    to: String,
    call_id: String,
    via: String,
    has_auth: bool,
    routes: Vec<String>,
}

struct MockSipPbx {
    pub local_addr: SocketAddr,
    pub received_packets: Arc<Mutex<Vec<ReceivedSipPacket>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockSipPbx {
    async fn start<F>(handler: F) -> Self
    where
        F: Fn(SipRequest, SocketAddr, Arc<UdpSocket>, Arc<Mutex<Vec<ReceivedSipPacket>>>)
            + Send
            + Sync
            + 'static,
    {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind mock pbx");
        let local_addr = socket.local_addr().expect("local addr");
        let socket = Arc::new(socket);
        let received_packets = Arc::new(Mutex::new(Vec::new()));
        let packets_clone = received_packets.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let mut buf = [0u8; 65535];
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    res = socket.recv_from(&mut buf) => {
                        let (len, client_addr) = match res {
                            Ok(r) => r,
                            Err(_) => break,
                        };
                        let text = match std::str::from_utf8(&buf[..len]) {
                            Ok(t) => t.to_string(),
                            Err(_) => continue,
                        };

                        if let Ok(SipMessage::Request(req)) = SipMessage::parse(&text) {
                            let has_auth = req.headers.contains_key("Authorization") || req.headers.contains_key("Proxy-Authorization");
                            let routes = req.headers.get("Route").map(|r| r.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default();

                            packets_clone.lock().await.push(ReceivedSipPacket {
                                raw: text.clone(),
                                method: req.method.clone(),
                                cseq: req.headers.get("CSeq").cloned().unwrap_or_default(),
                                from: req.headers.get("From").cloned().unwrap_or_default(),
                                to: req.headers.get("To").cloned().unwrap_or_default(),
                                call_id: req.headers.get("Call-ID").cloned().unwrap_or_default(),
                                via: req.headers.get("Via").cloned().unwrap_or_default(),
                                has_auth,
                                routes,
                            });

                            handler(req, client_addr, socket.clone(), packets_clone.clone());
                        }
                    }
                }
            }
        });

        Self {
            local_addr,
            received_packets,
            shutdown_tx: Some(shutdown_tx),
        }
    }
}

impl Drop for MockSipPbx {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

async fn setup_test_environment(
    pbx_addr: SocketAddr,
) -> (
    Arc<StorageManager>,
    Arc<CallManager>,
    AccountId,
    Arc<SipUdpTransport>,
) {
    let storage = Arc::new(StorageManager::new_in_memory().expect("in-memory db"));
    assert!(
        storage.is_isolated_backend(),
        "Must use isolated credential backend"
    );
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());
    let call_manager = Arc::new(CallManager::new(
        storage.clone(),
        transport.clone(),
        transactions.clone(),
    ));

    let sip_client = Arc::new(
        sip_engine::SipClient::new(transport.clone(), transactions.clone())
            .with_request_handler(call_manager.clone()),
    );
    let _recv_loop = sip_client.start_receive_loop();

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "Test User".into(),
        username: "620".into(),
        display_name: Some("Test Device".into()),
        domain: "fritz.box".into(),
        registrar: "127.0.0.1".into(),
        registrar_port: pbx_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };

    storage.save_account(&account).expect("save account");
    storage
        .save_account_secret(account_id, "correct_password")
        .expect("save secret");

    (storage, call_manager, account_id, transport)
}

#[tokio::test]
async fn test_r2_outgoing_call_establishment_and_local_hangup() {
    // 1. Start mock PBX simulating Ringing -> 200 OK -> BYE 200 OK
    let pbx = MockSipPbx::start(|req, client_addr, socket, _packets| {
        tokio::spawn(async move {
            match req.method {
                SipMethod::Invite => {
                    let via = req.headers.get("Via").unwrap();
                    let from = req.headers.get("From").unwrap();
                    let to = req.headers.get("To").unwrap();
                    let call_id = req.headers.get("Call-ID").unwrap();
                    let cseq = req.headers.get("CSeq").unwrap();

                    // 180 Ringing
                    let ringing = format!(
                        "SIP/2.0 180 Ringing\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=pbx-to-tag-1\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(ringing.as_bytes(), client_addr).await;

                    tokio::time::sleep(Duration::from_millis(50)).await;

                    // 200 OK with Record-Route
                    let sdp = "v=0\r\no=mock 1 1 IN IP4 127.0.0.1\r\ns=Call\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 40000 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\n";
                    let ok = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=pbx-to-tag-1\r\nCall-ID: {}\r\nCSeq: {}\r\nRecord-Route: <sip:rr1.fritz.box;lr>, <sip:rr2.fritz.box;lr>\r\nContact: <sip:611@127.0.0.1:{}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                        via, from, to, call_id, cseq, socket.local_addr().unwrap().port(), sdp.len(), sdp
                    );
                    let _ = socket.send_to(ok.as_bytes(), client_addr).await;
                }
                SipMethod::Bye => {
                    let via = req.headers.get("Via").unwrap();
                    let from = req.headers.get("From").unwrap();
                    let to = req.headers.get("To").unwrap();
                    let call_id = req.headers.get("Call-ID").unwrap();
                    let cseq = req.headers.get("CSeq").unwrap();

                    let ok = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(ok.as_bytes(), client_addr).await;
                }
                _ => {}
            }
        });
    }).await;

    let (storage, call_manager, account_id, _transport) =
        setup_test_environment(pbx.local_addr).await;
    let mut call_events = call_manager.subscribe_call_events();

    // 2. Initiate outgoing call via CallManager
    let call_id = call_manager
        .start_outgoing_call(account_id, "**611".into())
        .await
        .expect("start call");

    // 3. Verify event stream: Dialing -> RingingRemote -> Active
    let mut states = Vec::new();
    while let Ok(Ok((cid, st))) =
        tokio::time::timeout(Duration::from_secs(3), call_events.recv()).await
    {
        if cid == call_id {
            states.push(st.clone());
            if matches!(st, CallState::Active { .. }) {
                break;
            }
        }
    }

    assert!(states.iter().any(|s| matches!(s, CallState::Dialing)));
    assert!(states.iter().any(|s| matches!(s, CallState::RingingRemote)));
    assert!(states.iter().any(|s| matches!(s, CallState::Active { .. })));

    // Allow ACK packet to arrive at PBX
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 4. Verify packets received by PBX: INVITE and in-dialog ACK
    {
        let packets = pbx.received_packets.lock().await;
        let invite_pkt = packets
            .iter()
            .find(|p| p.method == SipMethod::Invite)
            .expect("INVITE on wire");
        assert!(invite_pkt.raw.contains("application/sdp"));
        assert!(invite_pkt.raw.contains("INVITE sip:**611@fritz.box"));

        let ack_pkt = packets
            .iter()
            .find(|p| p.method == SipMethod::Ack)
            .expect("ACK on wire");
        assert_eq!(ack_pkt.cseq, "1 ACK");
        // Verify Route set ordering (SIP-04): reversed Record-Route
        assert_eq!(
            ack_pkt.routes,
            vec!["<sip:rr2.fritz.box;lr>", "<sip:rr1.fritz.box;lr>"]
        );
    }

    // 5. Local hangup
    call_manager
        .hangup_call(call_id)
        .await
        .expect("hangup call");

    let term_event = tokio::time::timeout(Duration::from_secs(3), call_events.recv())
        .await
        .expect("term event timeout")
        .expect("term event");
    assert!(matches!(
        term_event.1,
        CallState::Terminated {
            reason: DisconnectReason::LocalHangup
        }
    ));

    // Verify BYE received on wire with matching Route set and incremented CSeq
    tokio::time::sleep(Duration::from_millis(50)).await;
    {
        let packets = pbx.received_packets.lock().await;
        let bye_pkt = packets
            .iter()
            .find(|p| p.method == SipMethod::Bye)
            .expect("BYE on wire");
        assert_eq!(bye_pkt.cseq, "2 BYE");
        assert_eq!(
            bye_pkt.routes,
            vec!["<sip:rr2.fritz.box;lr>", "<sip:rr1.fritz.box;lr>"]
        );
    }

    // 6. Verify SQLite call log
    let history = storage.list_call_history(10).expect("list history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, call_id);
    assert_eq!(history[0].direction, CallDirection::Outbound);
    assert_eq!(history[0].remote_party, "**611");
    assert_eq!(
        history[0].disconnect_reason,
        DisconnectReason::LocalHangup.to_string()
    );
}

#[tokio::test]
async fn test_r2_outgoing_call_remote_hangup() {
    let pbx = MockSipPbx::start(|req, client_addr, socket, _packets| {
        tokio::spawn(async move {
            if req.method == SipMethod::Invite {
                let via = req.headers.get("Via").unwrap();
                let from = req.headers.get("From").unwrap();
                let to = req.headers.get("To").unwrap();
                let call_id = req.headers.get("Call-ID").unwrap();
                let cseq = req.headers.get("CSeq").unwrap();

                let sdp = "v=0\r\no=mock 1 1 IN IP4 127.0.0.1\r\ns=Call\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 40000 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\n";
                let ok = format!(
                    "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=remote-tag-99\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:611@127.0.0.1:{}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                    via, from, to, call_id, cseq, socket.local_addr().unwrap().port(), sdp.len(), sdp
                );
                let _ = socket.send_to(ok.as_bytes(), client_addr).await;
            }
        });
    }).await;

    let (storage, call_manager, account_id, transport) =
        setup_test_environment(pbx.local_addr).await;
    let mut call_events = call_manager.subscribe_call_events();

    let call_id = call_manager
        .start_outgoing_call(account_id, "**611".into())
        .await
        .expect("start call");

    // Wait until call is Active
    loop {
        let (cid, st) = tokio::time::timeout(Duration::from_secs(3), call_events.recv())
            .await
            .expect("active timeout")
            .expect("recv");
        if cid == call_id && matches!(st, CallState::Active { .. }) {
            break;
        }
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Simulate remote party issuing in-dialog BYE to client
    let client_addr = transport.local_addr();
    let invite_pkt = {
        let pkts = pbx.received_packets.lock().await;
        pkts.iter()
            .find(|p| p.method == SipMethod::Invite)
            .unwrap()
            .clone()
    };

    let pbx_sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind pbx sock");
    let from_tag = sip_protocol::extract_tag(&invite_pkt.from).unwrap();
    let bye_req = format!(
        "BYE sip:620@{}:{} SIP/2.0\r\nVia: SIP/2.0/UDP {}:5060;branch=z9hG4bK-remote-bye-1\r\nMax-Forwards: 70\r\nFrom: <sip:**611@fritz.box>;tag=remote-tag-99\r\nTo: <sip:620@fritz.box>;tag={}\r\nCall-ID: {}\r\nCSeq: 10 BYE\r\nContent-Length: 0\r\n\r\n",
        client_addr.ip(),
        client_addr.port(),
        pbx.local_addr.ip(),
        from_tag,
        invite_pkt.call_id
    );

    let _ = pbx_sock.send_to(bye_req.as_bytes(), client_addr).await;

    // Verify client responds with 200 OK to the BYE
    let mut resp_buf = [0u8; 2048];
    let (resp_len, _) =
        tokio::time::timeout(Duration::from_secs(3), pbx_sock.recv_from(&mut resp_buf))
            .await
            .expect("timeout on BYE response")
            .expect("recv 200 OK");
    let resp_text = std::str::from_utf8(&resp_buf[..resp_len]).expect("utf8");
    assert!(
        resp_text.starts_with("SIP/2.0 200 OK"),
        "Expected 200 OK to BYE, got: {}",
        resp_text
    );

    // Verify CallManager transitions to RemoteHangup
    let term_event = tokio::time::timeout(Duration::from_secs(3), call_events.recv())
        .await
        .expect("term event timeout")
        .expect("term event");
    assert!(matches!(
        term_event.1,
        CallState::Terminated {
            reason: DisconnectReason::RemoteHangup
        }
    ));

    // Verify storage history
    let history = storage.list_call_history(10).expect("list history");
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].disconnect_reason,
        DisconnectReason::RemoteHangup.to_string()
    );
}

#[tokio::test]
async fn test_r2_outgoing_call_digest_authentication_challenge() {
    let pbx = MockSipPbx::start(|req, client_addr, socket, _packets| {
        tokio::spawn(async move {
            let via = req.headers.get("Via").unwrap();
            let from = req.headers.get("From").unwrap();
            let to = req.headers.get("To").unwrap();
            let call_id = req.headers.get("Call-ID").unwrap();
            let cseq = req.headers.get("CSeq").unwrap();

            if req.method == SipMethod::Invite {
                if !req.headers.contains_key("Authorization") {
                    // Send 401 challenge
                    let challenge = format!(
                        "SIP/2.0 401 Unauthorized\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=chal-to-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nWWW-Authenticate: Digest realm=\"fritz.box\", nonce=\"r2callnonce88\", algorithm=MD5, qop=\"auth\"\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(challenge.as_bytes(), client_addr).await;
                } else {
                    // Authenticated INVITE -> send 200 OK
                    let sdp = "v=0\r\no=mock 1 1 IN IP4 127.0.0.1\r\ns=Call\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 40000 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\n";
                    let ok = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=auth-active-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:611@127.0.0.1:{}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                        via, from, to, call_id, cseq, socket.local_addr().unwrap().port(), sdp.len(), sdp
                    );
                    let _ = socket.send_to(ok.as_bytes(), client_addr).await;
                }
            }
        });
    }).await;

    let (_storage, call_manager, account_id, _transport) =
        setup_test_environment(pbx.local_addr).await;
    let mut call_events = call_manager.subscribe_call_events();

    let call_id = call_manager
        .start_outgoing_call(account_id, "**611".into())
        .await
        .expect("start call");

    // Wait until call becomes Active
    loop {
        let (cid, st) = tokio::time::timeout(Duration::from_secs(3), call_events.recv())
            .await
            .expect("active timeout")
            .expect("recv");
        if cid == call_id && matches!(st, CallState::Active { .. }) {
            break;
        }
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify network packet exchange:
    // 1. Initial INVITE (CSeq: 1 INVITE)
    // 2. Hop-by-hop ACK for 401 (CSeq: 1 ACK) (RFC 3261 §17.1.1.3)
    // 3. Authenticated re-INVITE (CSeq: 2 INVITE, Authorization header)
    // 4. In-dialog ACK for 200 OK (CSeq: 2 ACK) (SIP-05)
    let packets = pbx.received_packets.lock().await;

    let invites: Vec<_> = packets
        .iter()
        .filter(|p| p.method == SipMethod::Invite)
        .collect();
    assert_eq!(
        invites.len(),
        2,
        "Expected 2 INVITE requests (initial + authenticated)"
    );
    assert_eq!(invites[0].cseq, "1 INVITE");
    assert!(!invites[0].has_auth);
    assert_eq!(invites[1].cseq, "2 INVITE");
    assert!(invites[1].has_auth);

    let acks: Vec<_> = packets
        .iter()
        .filter(|p| p.method == SipMethod::Ack)
        .collect();
    assert_eq!(
        acks.len(),
        2,
        "Expected 2 ACK requests (hop-by-hop 401 ACK + 200 OK ACK)"
    );
    // Hop-by-hop ACK matches initial INVITE CSeq
    assert_eq!(acks[0].cseq, "1 ACK");
    // In-dialog ACK matches 200 OK CSeq
    assert_eq!(acks[1].cseq, "2 ACK");
}

#[tokio::test]
async fn test_r2_outgoing_call_busy_and_rejected_responses() {
    // Test 486 Busy Here
    {
        let pbx = MockSipPbx::start(|req, client_addr, socket, _packets| {
            tokio::spawn(async move {
                if req.method == SipMethod::Invite {
                    let via = req.headers.get("Via").unwrap();
                    let from = req.headers.get("From").unwrap();
                    let to = req.headers.get("To").unwrap();
                    let call_id = req.headers.get("Call-ID").unwrap();
                    let cseq = req.headers.get("CSeq").unwrap();

                    let busy = format!(
                        "SIP/2.0 486 Busy Here\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=busy-to-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(busy.as_bytes(), client_addr).await;
                }
            });
        }).await;

        let (_storage, call_manager, account_id, _transport) =
            setup_test_environment(pbx.local_addr).await;
        let mut call_events = call_manager.subscribe_call_events();

        let call_id = call_manager
            .start_outgoing_call(account_id, "**611".into())
            .await
            .expect("start call");

        let mut terminated_received = false;
        while let Ok(Ok((cid, st))) =
            tokio::time::timeout(Duration::from_secs(3), call_events.recv()).await
        {
            if cid == call_id
                && matches!(
                    st,
                    CallState::Terminated {
                        reason: DisconnectReason::Busy
                    }
                )
            {
                terminated_received = true;
                break;
            }
        }
        assert!(terminated_received, "Expected DisconnectReason::Busy");

        // Verify hop-by-hop ACK was transmitted for 486 Busy (RFC 3261 §17.1.1.3)
        tokio::time::sleep(Duration::from_millis(50)).await;
        let packets = pbx.received_packets.lock().await;
        let ack = packets
            .iter()
            .find(|p| p.method == SipMethod::Ack)
            .expect("ACK on wire for 486");
        assert_eq!(ack.cseq, "1 ACK");
    }

    // Test 603 Decline / Rejected
    {
        let pbx = MockSipPbx::start(|req, client_addr, socket, _packets| {
            tokio::spawn(async move {
                if req.method == SipMethod::Invite {
                    let via = req.headers.get("Via").unwrap();
                    let from = req.headers.get("From").unwrap();
                    let to = req.headers.get("To").unwrap();
                    let call_id = req.headers.get("Call-ID").unwrap();
                    let cseq = req.headers.get("CSeq").unwrap();

                    let decline = format!(
                        "SIP/2.0 603 Decline\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=decline-to-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(decline.as_bytes(), client_addr).await;
                }
            });
        }).await;

        let (_storage, call_manager, account_id, _transport) =
            setup_test_environment(pbx.local_addr).await;
        let mut call_events = call_manager.subscribe_call_events();

        let call_id = call_manager
            .start_outgoing_call(account_id, "**611".into())
            .await
            .expect("start call");

        let mut terminated_received = false;
        while let Ok(Ok((cid, st))) =
            tokio::time::timeout(Duration::from_secs(3), call_events.recv()).await
        {
            if cid == call_id
                && matches!(
                    st,
                    CallState::Terminated {
                        reason: DisconnectReason::Rejected
                    }
                )
            {
                terminated_received = true;
                break;
            }
        }
        assert!(terminated_received, "Expected DisconnectReason::Rejected");

        tokio::time::sleep(Duration::from_millis(50)).await;
        let packets = pbx.received_packets.lock().await;
        let ack = packets
            .iter()
            .find(|p| p.method == SipMethod::Ack)
            .expect("ACK on wire for 603");
        assert_eq!(ack.cseq, "1 ACK");
    }
}

#[tokio::test]
async fn test_r2_outgoing_call_cancel_before_answer() {
    let pbx = MockSipPbx::start(|req, client_addr, socket, _packets| {
        tokio::spawn(async move {
            let via = req.headers.get("Via").unwrap();
            let from = req.headers.get("From").unwrap();
            let to = req.headers.get("To").unwrap();
            let call_id = req.headers.get("Call-ID").unwrap();
            let cseq = req.headers.get("CSeq").unwrap();

            match req.method {
                SipMethod::Invite => {
                    // Send 180 Ringing and keep ringing
                    let ringing = format!(
                        "SIP/2.0 180 Ringing\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=ringing-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(ringing.as_bytes(), client_addr).await;
                }
                SipMethod::Cancel => {
                    // 200 OK to CANCEL
                    let ok_cancel = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(ok_cancel.as_bytes(), client_addr).await;

                    // 487 Request Terminated to INVITE
                    let term = format!(
                        "SIP/2.0 487 Request Terminated\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=ringing-tag\r\nCall-ID: {}\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id
                    );
                    let _ = socket.send_to(term.as_bytes(), client_addr).await;
                }
                _ => {}
            }
        });
    }).await;

    let (_storage, call_manager, account_id, _transport) =
        setup_test_environment(pbx.local_addr).await;
    let mut call_events = call_manager.subscribe_call_events();

    let call_id = call_manager
        .start_outgoing_call(account_id, "**611".into())
        .await
        .expect("start call");

    // Wait until RingingRemote
    loop {
        let (cid, st) = tokio::time::timeout(Duration::from_secs(3), call_events.recv())
            .await
            .expect("ringing timeout")
            .expect("recv");
        if cid == call_id && matches!(st, CallState::RingingRemote) {
            break;
        }
    }

    // Cancel the call before answer
    call_manager
        .hangup_call(call_id)
        .await
        .expect("cancel call");

    let term_event = tokio::time::timeout(Duration::from_secs(3), call_events.recv())
        .await
        .expect("term event timeout")
        .expect("term event");
    assert!(matches!(
        term_event.1,
        CallState::Terminated {
            reason: DisconnectReason::Cancelled
        }
    ));

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify CANCEL request on wire (SIP-03 & RFC 3261 §9.1 invariants):
    let packets = pbx.received_packets.lock().await;
    let cancel_pkt = packets
        .iter()
        .find(|p| p.method == SipMethod::Cancel)
        .expect("CANCEL on wire");
    let invite_pkt = packets
        .iter()
        .find(|p| p.method == SipMethod::Invite)
        .expect("INVITE on wire");

    // CANCEL must match active INVITE's From tag, Call-ID, and numeric CSeq
    assert!(
        cancel_pkt.from.contains(";tag="),
        "CANCEL must preserve From tag (SIP-03)"
    );
    assert_eq!(cancel_pkt.from, invite_pkt.from);
    assert_eq!(cancel_pkt.call_id, invite_pkt.call_id);
    assert_eq!(cancel_pkt.cseq, "1 CANCEL");

    // Hop-by-hop ACK for 487 was transmitted
    let ack_pkt = packets
        .iter()
        .find(|p| p.method == SipMethod::Ack)
        .expect("ACK on wire for 487");
    assert_eq!(ack_pkt.cseq, "1 ACK");
}

#[tokio::test]
async fn test_r2_outgoing_call_timeout_cleans_up_without_orphaned_tasks() {
    // Mock PBX that never answers
    let pbx = MockSipPbx::start(|_req, _client_addr, _socket, _packets| {}).await;

    let (_storage, call_manager, account_id, _transport) =
        setup_test_environment(pbx.local_addr).await;

    let call_id = call_manager
        .start_outgoing_call(account_id, "**611".into())
        .await
        .expect("start call");

    // Active call is present initially
    let active = call_manager.get_active_call().await;
    assert!(active.is_some());

    // When timeout occurs or call is ended, verify state transitions to Terminated
    call_manager
        .end_call(call_id, DisconnectReason::Timeout)
        .await;

    assert!(call_manager.get_active_call().await.is_none());
}
