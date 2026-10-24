//! Milestone 9: Generic SIP Interoperability Test against a controlled RFC 3261 PBX/registrar endpoint.
//! Validates provider independence (non-FRITZ!Box server with PCMU, generic realm, and standard SIP routing).

use sip_domain::{
    AccountId, CallState, DisconnectReason, RegistrationState, SipAccount, TransportProtocol,
};
use sip_engine::call::OutgoingCallSession;
use sip_engine::registration::RegistrationManager;
use sip_protocol::{
    SdpSession, SipMessage, SipMethod, SipResponse, TransactionManager, generate_tag,
};
use sip_providers::{GenericSipProvider, SipProviderProfile};
use sip_transport::SipUdpTransport;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_generic_sip_pbx_interoperability_full_cycle() {
    // 1. Setup mock generic SIP PBX (e.g. Asterisk / Kamailio simulation)
    let pbx_socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind pbx socket");
    let pbx_addr = pbx_socket.local_addr().unwrap();

    // 2. Setup Client Transport and Transaction Manager
    let client_transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    // 3. Start client receive loop
    let rx_transport = Arc::clone(&client_transport);
    let rx_txs = Arc::clone(&transactions);
    let _rx_handle = tokio::spawn(async move {
        let mut buf = [0u8; 65535];
        while let Ok((len, _)) = rx_transport.recv_from(&mut buf).await {
            if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                if let Ok(SipMessage::Response(res)) = SipMessage::parse(text) {
                    rx_txs.handle_response(res).await;
                }
            }
        }
    });

    // 4. Configure Generic SIP Account
    let account = SipAccount {
        id: AccountId::new(),
        name: "Generic Asterisk".into(),
        username: "user101".into(),
        display_name: Some("Alice Office".into()),
        domain: "sip.example.com".into(),
        registrar: pbx_addr.ip().to_string(),
        registrar_port: pbx_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };

    // Verify provider dial normalization for generic SIP
    let provider = GenericSipProvider;
    let normalized = provider
        .normalize_dial_target("+15551234567", &account)
        .unwrap();
    assert_eq!(
        normalized.normalized_sip_uri,
        "sip:+15551234567@sip.example.com"
    );

    // 5. Run Registration Sequence against Generic PBX
    let reg_mgr = RegistrationManager::new(
        account.clone(),
        "password101".into(),
        Arc::clone(&client_transport),
        Arc::clone(&transactions),
    );

    // Spawn PBX handling task for registration challenge-response
    let pbx_task = tokio::spawn(async move {
        let mut buf = [0u8; 2048];

        // First REGISTER (no credentials)
        let (len1, from1) = pbx_socket.recv_from(&mut buf).await.unwrap();
        let msg1 = SipMessage::parse(std::str::from_utf8(&buf[..len1]).unwrap()).unwrap();
        if let SipMessage::Request(req) = msg1 {
            assert_eq!(req.method, SipMethod::Register);
            let mut headers = HashMap::new();
            headers.insert("Via".into(), req.headers.get("Via").unwrap().clone());
            headers.insert("From".into(), req.headers.get("From").unwrap().clone());
            headers.insert(
                "To".into(),
                format!("{};tag=pbx_tag_1", req.headers.get("To").unwrap()),
            );
            headers.insert(
                "Call-ID".into(),
                req.headers.get("Call-ID").unwrap().clone(),
            );
            headers.insert("CSeq".into(), req.headers.get("CSeq").unwrap().clone());
            headers.insert(
                "WWW-Authenticate".into(),
                "Digest realm=\"sip.example.com\", nonce=\"nonce_xyz789\", algorithm=MD5".into(),
            );

            let res401 = SipResponse {
                status_code: 401,
                reason_phrase: "Unauthorized".into(),
                headers,
                raw_headers: Vec::new(),
                body: String::new(),
            };
            pbx_socket
                .send_to(res401.serialize().as_bytes(), from1)
                .await
                .unwrap();
        }

        // Second REGISTER (with credentials)
        let (len2, from2) = pbx_socket.recv_from(&mut buf).await.unwrap();
        let msg2 = SipMessage::parse(std::str::from_utf8(&buf[..len2]).unwrap()).unwrap();
        if let SipMessage::Request(req) = msg2 {
            assert_eq!(req.method, SipMethod::Register);
            assert!(req.headers.contains_key("Authorization"));

            let mut headers = HashMap::new();
            headers.insert("Via".into(), req.headers.get("Via").unwrap().clone());
            headers.insert("From".into(), req.headers.get("From").unwrap().clone());
            headers.insert(
                "To".into(),
                format!("{};tag=pbx_tag_2", req.headers.get("To").unwrap()),
            );
            headers.insert(
                "Call-ID".into(),
                req.headers.get("Call-ID").unwrap().clone(),
            );
            headers.insert("CSeq".into(), req.headers.get("CSeq").unwrap().clone());
            headers.insert(
                "Contact".into(),
                req.headers.get("Contact").unwrap().clone(),
            );

            let res200 = SipResponse {
                status_code: 200,
                reason_phrase: "OK".into(),
                headers,
                raw_headers: Vec::new(),
                body: String::new(),
            };
            pbx_socket
                .send_to(res200.serialize().as_bytes(), from2)
                .await
                .unwrap();
        }

        pbx_socket
    });

    reg_mgr
        .register(pbx_addr)
        .await
        .expect("registration succeeds");
    assert!(matches!(
        reg_mgr.get_state().await,
        RegistrationState::Registered { .. }
    ));

    let pbx_socket = pbx_task.await.unwrap();

    // 6. Test Outgoing Call to Generic Destination with G.711 μ-law (PCMU)
    let (call_event_tx, _call_event_rx) = mpsc::channel(16);
    let call_session = OutgoingCallSession::new(
        account.clone(),
        "password101".into(),
        "sip:user102@sip.example.com".into(),
        14000,
        Arc::clone(&client_transport),
        Arc::clone(&transactions),
        call_event_tx,
    );

    let pbx_call_task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];

        // Receive INVITE with SDP offer
        let (len, from) = pbx_socket.recv_from(&mut buf).await.unwrap();
        let msg = SipMessage::parse(std::str::from_utf8(&buf[..len]).unwrap()).unwrap();
        if let SipMessage::Request(req) = msg {
            assert_eq!(req.method, SipMethod::Invite);

            // Respond 100 Trying
            let trying = SipResponse::create_for_request(&req, 100, "Trying", None, None, None);
            pbx_socket
                .send_to(trying.serialize().as_bytes(), from)
                .await
                .unwrap();

            // Respond 180 Ringing
            let to_tag = generate_tag();
            let ringing =
                SipResponse::create_for_request(&req, 180, "Ringing", Some(&to_tag), None, None);
            pbx_socket
                .send_to(ringing.serialize().as_bytes(), from)
                .await
                .unwrap();

            // Respond 200 OK with PCMU (0) SDP Answer
            let sdp_answer =
                SdpSession::new(pbx_socket.local_addr().unwrap().ip(), 14002, vec![0, 101]);
            let ok = SipResponse::create_for_request(
                &req,
                200,
                "OK",
                Some(&to_tag),
                Some(&format!(
                    "<sip:user102@{}>",
                    pbx_socket.local_addr().unwrap()
                )),
                Some(sdp_answer.serialize()),
            );
            pbx_socket
                .send_to(ok.serialize().as_bytes(), from)
                .await
                .unwrap();

            // Receive ACK
            let (ack_len, _) = pbx_socket.recv_from(&mut buf).await.unwrap();
            let ack_msg = SipMessage::parse(std::str::from_utf8(&buf[..ack_len]).unwrap()).unwrap();
            if let SipMessage::Request(ack_req) = ack_msg {
                assert_eq!(ack_req.method, SipMethod::Ack);
            }

            // Receive BYE
            let (bye_len, bye_from) = pbx_socket.recv_from(&mut buf).await.unwrap();
            let bye_msg = SipMessage::parse(std::str::from_utf8(&buf[..bye_len]).unwrap()).unwrap();
            if let SipMessage::Request(bye_req) = bye_msg {
                assert_eq!(bye_req.method, SipMethod::Bye);
                let bye_ok = SipResponse::create_for_request(&bye_req, 200, "OK", None, None, None);
                pbx_socket
                    .send_to(bye_ok.serialize().as_bytes(), bye_from)
                    .await
                    .unwrap();
            }
        }
    });

    call_session
        .start(pbx_addr)
        .await
        .expect("start outgoing call");
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(matches!(
        call_session.get_state().await,
        CallState::Active { .. }
    ));

    // Hangup call
    call_session
        .hangup(pbx_addr)
        .await
        .expect("hangup outgoing call");
    assert!(matches!(
        call_session.get_state().await,
        CallState::Terminated {
            reason: DisconnectReason::LocalHangup
        }
    ));

    pbx_call_task.await.unwrap();
}
