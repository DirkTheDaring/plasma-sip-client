use sip_domain::{AudioCodec, CallState, DisconnectReason};
use sip_engine::incoming::IncomingCallSession;
use sip_protocol::{SdpSession, SipMessage, SipMethod, SipRequest, generate_branch, generate_tag};
use sip_transport::SipUdpTransport;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

fn build_invite_request(
    remote_addr: SocketAddr,
    local_uas_addr: SocketAddr,
    call_id: &str,
    from_tag: &str,
    branch: &str,
    remote_rtp_port: u16,
) -> SipRequest {
    let mut headers = HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", remote_addr, branch),
    );
    headers.insert("Max-Forwards".into(), "70".into());
    headers.insert(
        "From".into(),
        format!("<sip:alice@{}>;tag={}", remote_addr, from_tag),
    );
    headers.insert("To".into(), format!("<sip:bob@{}>", local_uas_addr));
    headers.insert("Call-ID".into(), call_id.to_string());
    headers.insert("CSeq".into(), "100 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:alice@{}>", remote_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let sdp = SdpSession::new(remote_addr.ip(), remote_rtp_port, vec![8, 0, 101]);
    let body = sdp.serialize();

    SipRequest {
        method: SipMethod::Invite,
        ruri: format!("sip:bob@{}", local_uas_addr),
        headers,
        raw_headers: Vec::new(),
        body,
    }
}

#[tokio::test]
async fn test_incoming_call_answered_and_hangup() {
    // 1. Setup client UAS transport
    let uas_transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind uas transport"),
    );
    let uas_addr = uas_transport.local_addr();

    // 2. Setup mock remote UAC socket
    let remote_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind remote socket");
    let remote_addr = remote_sock.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(16);
    let call_id = "inc-call-100@remote.net";
    let from_tag = generate_tag();
    let branch = generate_branch();

    let invite = build_invite_request(remote_addr, uas_addr, call_id, &from_tag, &branch, 12000);

    // 3. UAS receives incoming INVITE and initializes session
    let uas_session = IncomingCallSession::new_and_ring(
        invite.clone(),
        remote_addr,
        Arc::clone(&uas_transport),
        event_tx,
    )
    .await
    .expect("new and ring");

    assert_eq!(uas_session.get_state().await, CallState::RingingLocal);

    // 4. Remote receives 100 Trying then 180 Ringing
    let mut buf = [0u8; 2048];
    let (len1, _) = remote_sock.recv_from(&mut buf).await.expect("recv trying");
    let msg1 = SipMessage::parse(std::str::from_utf8(&buf[..len1]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg1 {
        assert_eq!(res.status_code, 100);
    } else {
        panic!("Expected 100 Trying");
    }

    let (len2, _) = remote_sock.recv_from(&mut buf).await.expect("recv ringing");
    let msg2 = SipMessage::parse(std::str::from_utf8(&buf[..len2]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg2 {
        assert_eq!(res.status_code, 180);
        assert!(res.headers.get("To").unwrap().contains(";tag="));
        assert!(
            res.headers.contains_key("Contact"),
            "RFC 3261 early dialog 180 Ringing MUST have Contact"
        );
    } else {
        panic!("Expected 180 Ringing");
    }

    // 5. UAS user answers call
    uas_session
        .answer(12002, AudioCodec::Pcma)
        .await
        .expect("answer call");

    assert!(matches!(
        uas_session.get_state().await,
        CallState::Active { .. }
    ));

    // 6. Remote receives 200 OK with SDP answer
    let (len3, _) = remote_sock.recv_from(&mut buf).await.expect("recv 200 OK");
    let msg3 = SipMessage::parse(std::str::from_utf8(&buf[..len3]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg3 {
        assert_eq!(res.status_code, 200);
        let sdp = SdpSession::parse(&res.body).expect("parse SDP answer");
        assert_eq!(sdp.audio_port, 12002);
        assert!(sdp.payload_types.contains(&8));
    } else {
        panic!("Expected 200 OK");
    }

    // 7. UAS user hangs up
    uas_session.hangup().await.expect("hangup");
    assert!(matches!(
        uas_session.get_state().await,
        CallState::Terminated {
            reason: DisconnectReason::LocalHangup
        }
    ));

    // 8. Remote receives BYE
    let (len4, _) = remote_sock.recv_from(&mut buf).await.expect("recv BYE");
    let msg4 = SipMessage::parse(std::str::from_utf8(&buf[..len4]).unwrap()).unwrap();
    if let SipMessage::Request(req) = msg4 {
        assert_eq!(req.method, SipMethod::Bye);
    } else {
        panic!("Expected BYE");
    }
}

#[tokio::test]
async fn test_incoming_call_rejected() {
    let uas_transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind uas transport"),
    );
    let uas_addr = uas_transport.local_addr();

    let remote_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind remote socket");
    let remote_addr = remote_sock.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(16);
    let call_id = "inc-call-reject@remote.net";
    let from_tag = generate_tag();
    let branch = generate_branch();

    let invite = build_invite_request(remote_addr, uas_addr, call_id, &from_tag, &branch, 12010);

    let uas_session = IncomingCallSession::new_and_ring(
        invite,
        remote_addr,
        Arc::clone(&uas_transport),
        event_tx,
    )
    .await
    .expect("new and ring");

    // Flush trying & ringing
    let mut buf = [0u8; 2048];
    let _ = remote_sock.recv_from(&mut buf).await;
    let _ = remote_sock.recv_from(&mut buf).await;

    // Reject call with 603 Decline
    uas_session.reject(603, "Decline").await.expect("reject");

    assert!(matches!(
        uas_session.get_state().await,
        CallState::Terminated {
            reason: DisconnectReason::Rejected
        }
    ));

    let (len, _) = remote_sock.recv_from(&mut buf).await.expect("recv 603");
    let msg = SipMessage::parse(std::str::from_utf8(&buf[..len]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg {
        assert_eq!(res.status_code, 603);
    } else {
        panic!("Expected 603 Decline");
    }
}

#[tokio::test]
async fn test_incoming_call_busy() {
    let uas_transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind uas transport"),
    );
    let uas_addr = uas_transport.local_addr();

    let remote_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind remote socket");
    let remote_addr = remote_sock.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(16);
    let call_id = "inc-call-busy@remote.net";
    let from_tag = generate_tag();
    let branch = generate_branch();

    let invite = build_invite_request(remote_addr, uas_addr, call_id, &from_tag, &branch, 12015);

    let uas_session = IncomingCallSession::new_and_ring(
        invite,
        remote_addr,
        Arc::clone(&uas_transport),
        event_tx,
    )
    .await
    .expect("new and ring");

    // Flush trying & ringing
    let mut buf = [0u8; 2048];
    let _ = remote_sock.recv_from(&mut buf).await;
    let _ = remote_sock.recv_from(&mut buf).await;

    // Reject with 486 Busy Here
    uas_session
        .reject(486, "Busy Here")
        .await
        .expect("reject busy");

    assert!(matches!(
        uas_session.get_state().await,
        CallState::Terminated {
            reason: DisconnectReason::Busy
        }
    ));

    let (len, _) = remote_sock.recv_from(&mut buf).await.expect("recv 486");
    let msg = SipMessage::parse(std::str::from_utf8(&buf[..len]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg {
        assert_eq!(res.status_code, 486);
    } else {
        panic!("Expected 486 Busy Here");
    }
}

#[tokio::test]
async fn test_incoming_call_remote_cancel() {
    let uas_transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind uas transport"),
    );
    let uas_addr = uas_transport.local_addr();

    let remote_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind remote socket");
    let remote_addr = remote_sock.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(16);
    let call_id = "inc-call-cancel@remote.net";
    let from_tag = generate_tag();
    let branch = generate_branch();

    let invite = build_invite_request(remote_addr, uas_addr, call_id, &from_tag, &branch, 12020);

    let uas_session = IncomingCallSession::new_and_ring(
        invite.clone(),
        remote_addr,
        Arc::clone(&uas_transport),
        event_tx,
    )
    .await
    .expect("new and ring");

    // Flush trying & ringing
    let mut buf = [0u8; 2048];
    let _ = remote_sock.recv_from(&mut buf).await;
    let _ = remote_sock.recv_from(&mut buf).await;

    // Remote transmits CANCEL matching branch
    let mut cancel_headers = HashMap::new();
    cancel_headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", remote_addr, branch),
    );
    cancel_headers.insert(
        "From".into(),
        format!("<sip:alice@{}>;tag={}", remote_addr, from_tag),
    );
    cancel_headers.insert("To".into(), format!("<sip:bob@{}>", uas_addr));
    cancel_headers.insert("Call-ID".into(), call_id.to_string());
    cancel_headers.insert("CSeq".into(), "100 CANCEL".into());

    let cancel_req = SipRequest {
        method: SipMethod::Cancel,
        ruri: format!("sip:bob@{}", uas_addr),
        headers: cancel_headers,
        raw_headers: Vec::new(),
        body: String::new(),
    };

    uas_session
        .handle_cancel(&cancel_req)
        .await
        .expect("handle cancel");

    assert!(matches!(
        uas_session.get_state().await,
        CallState::Terminated {
            reason: DisconnectReason::Cancelled
        }
    ));

    // Remote receives 200 OK to CANCEL
    let (len1, _) = remote_sock
        .recv_from(&mut buf)
        .await
        .expect("recv 200 OK to CANCEL");
    let msg1 = SipMessage::parse(std::str::from_utf8(&buf[..len1]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg1 {
        assert_eq!(res.status_code, 200);
        assert_eq!(res.headers.get("CSeq").unwrap(), "100 CANCEL");
    } else {
        panic!("Expected 200 OK to CANCEL");
    }

    // Remote receives 487 Request Terminated to INVITE
    let (len2, _) = remote_sock
        .recv_from(&mut buf)
        .await
        .expect("recv 487 to INVITE");
    let msg2 = SipMessage::parse(std::str::from_utf8(&buf[..len2]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg2 {
        assert_eq!(res.status_code, 487);
        assert_eq!(res.headers.get("CSeq").unwrap(), "100 INVITE");
    } else {
        panic!("Expected 487 Request Terminated");
    }

    // Retransmitted CANCEL should be handled idempotently with 200 OK
    uas_session
        .handle_cancel(&cancel_req)
        .await
        .expect("handle retransmitted cancel idempotently");

    let (len3, _) = remote_sock
        .recv_from(&mut buf)
        .await
        .expect("recv 200 OK to retransmitted CANCEL");
    let msg3 = SipMessage::parse(std::str::from_utf8(&buf[..len3]).unwrap()).unwrap();
    if let SipMessage::Response(res) = msg3 {
        assert_eq!(res.status_code, 200);
        assert_eq!(res.headers.get("CSeq").unwrap(), "100 CANCEL");
    } else {
        panic!("Expected 200 OK to retransmitted CANCEL");
    }
}
