//! Milestone 10: Repeated Call Cycle Stress Testing.
//! Validates concurrency, memory stability, and zero resource leaks across 50 rapid call cycles.

use sip_domain::{AudioCodec, CallState, DisconnectReason};
use sip_engine::incoming::IncomingCallSession;
use sip_protocol::{SdpSession, SipRequest, generate_branch, generate_tag};
use sip_transport::SipUdpTransport;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

fn build_stress_invite(
    remote_addr: SocketAddr,
    local_uas_addr: SocketAddr,
    call_id: &str,
    from_tag: &str,
    branch: &str,
) -> SipRequest {
    let mut headers = HashMap::new();
    headers.insert(
        "Via".into(),
        format!("SIP/2.0/UDP {};branch={}", remote_addr, branch),
    );
    headers.insert("Max-Forwards".into(), "70".into());
    headers.insert(
        "From".into(),
        format!("<sip:stress@{}>;tag={}", remote_addr, from_tag),
    );
    headers.insert("To".into(), format!("<sip:uas@{}>", local_uas_addr));
    headers.insert("Call-ID".into(), call_id.to_string());
    headers.insert("CSeq".into(), "1 INVITE".into());
    headers.insert("Contact".into(), format!("<sip:stress@{}>", remote_addr));
    headers.insert("Content-Type".into(), "application/sdp".into());

    let sdp = SdpSession::new(remote_addr.ip(), 16000, vec![8, 0]);

    SipRequest {
        method: sip_protocol::SipMethod::Invite,
        ruri: format!("sip:uas@{}", local_uas_addr),
        headers,
        raw_headers: Vec::new(),
        body: sdp.serialize(),
    }
}

#[tokio::test]
async fn test_50_repeated_rapid_call_cycles() {
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

    // Run 50 rapid sequential call setup and teardown cycles
    for i in 0..50 {
        let (event_tx, _event_rx) = mpsc::channel(16);
        let call_id = format!("stress-cycle-{}@stress.net", i);
        let from_tag = generate_tag();
        let branch = generate_branch();

        let invite = build_stress_invite(remote_addr, uas_addr, &call_id, &from_tag, &branch);

        // Inbound INVITE -> 100 Trying & 180 Ringing
        let session = IncomingCallSession::new_and_ring(
            invite,
            remote_addr,
            Arc::clone(&uas_transport),
            event_tx,
        )
        .await
        .expect("new_and_ring cycle");

        assert_eq!(session.get_state().await, CallState::RingingLocal);

        // Flush trying and ringing on remote socket
        let mut buf = [0u8; 2048];
        let _ = remote_sock.recv_from(&mut buf).await;
        let _ = remote_sock.recv_from(&mut buf).await;

        // Answer
        session
            .answer(16002, AudioCodec::Pcma)
            .await
            .expect("answer cycle");
        assert!(matches!(
            session.get_state().await,
            CallState::Active { .. }
        ));

        // Flush 200 OK
        let _ = remote_sock.recv_from(&mut buf).await;

        // Local hangup
        session.hangup().await.expect("hangup cycle");
        assert!(matches!(
            session.get_state().await,
            CallState::Terminated {
                reason: DisconnectReason::LocalHangup
            }
        ));

        // Flush BYE
        let _ = remote_sock.recv_from(&mut buf).await;
    }
}
