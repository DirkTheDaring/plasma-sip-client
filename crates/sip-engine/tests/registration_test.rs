//! End-to-end SIP Registration Integration Tests against a local automated registrar fixture.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;

use sip_domain::{AccountId, RegistrationState, SipAccount, TransportProtocol};
use sip_engine::RegistrationManager;
use sip_protocol::{SipMessage, SipMethod, TransactionManager};
use sip_transport::SipUdpTransport;

/// Simulated SIP Registrar running on localhost for automated protocol testing.
struct MockSipRegistrar {
    local_addr: SocketAddr,
}

impl MockSipRegistrar {
    async fn start(require_auth: bool, expected_password: Option<&str>) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind mock registrar");
        let local_addr = socket.local_addr().expect("local addr");
        let socket = Arc::new(socket);

        let registrar = Self { local_addr };

        // Spawn mock registrar handling loop
        let sock_clone = socket.clone();
        let exp_pw = expected_password.map(|s| s.to_string());
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                let (len, client_addr) = match sock_clone.recv_from(&mut buf).await {
                    Ok(res) => res,
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

                if msg.method == SipMethod::Register {
                    let has_auth = msg.headers.contains_key("Authorization");
                    let via = msg.headers.get("Via").cloned().unwrap_or_default();
                    let from = msg.headers.get("From").cloned().unwrap_or_default();
                    let to = msg.headers.get("To").cloned().unwrap_or_default();
                    let call_id = msg.headers.get("Call-ID").cloned().unwrap_or_default();
                    let cseq = msg.headers.get("CSeq").cloned().unwrap_or_default();

                    if require_auth && !has_auth {
                        // Respond 401 Unauthorized with Digest challenge
                        let resp = format!(
                            "SIP/2.0 401 Unauthorized\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=reg-srv-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nWWW-Authenticate: Digest realm=\"fritz.box\", nonce=\"mocknonce123\", algorithm=MD5, qop=\"auth\"\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq
                        );
                        let _ = sock_clone.send_to(resp.as_bytes(), client_addr).await;
                    } else if require_auth && has_auth {
                        // Check if password / auth is accepted
                        let is_valid = if let Some(ref pw) = exp_pw {
                            // If wrong password test, reject
                            pw != "wrongpassword"
                        } else {
                            true
                        };

                        let expires_val = msg
                            .headers
                            .get("Expires")
                            .cloned()
                            .unwrap_or_else(|| "300".into());

                        if is_valid {
                            let resp = format!(
                                "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=reg-srv-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:620@{}:{}>;expires={}\r\nExpires: {}\r\nContent-Length: 0\r\n\r\n",
                                via,
                                from,
                                to,
                                call_id,
                                cseq,
                                client_addr.ip(),
                                client_addr.port(),
                                expires_val,
                                expires_val
                            );
                            let _ = sock_clone.send_to(resp.as_bytes(), client_addr).await;
                        } else {
                            let resp = format!(
                                "SIP/2.0 403 Forbidden\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=reg-srv-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                                via, from, to, call_id, cseq
                            );
                            let _ = sock_clone.send_to(resp.as_bytes(), client_addr).await;
                        }
                    } else {
                        let expires_val = msg
                            .headers
                            .get("Expires")
                            .cloned()
                            .unwrap_or_else(|| "300".into());
                        // Direct 200 OK without authentication
                        let resp = format!(
                            "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=reg-srv-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nExpires: {}\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq, expires_val
                        );
                        let _ = sock_clone.send_to(resp.as_bytes(), client_addr).await;
                    }
                }
            }
        });

        registrar
    }
}

fn create_test_account(registrar_addr: SocketAddr) -> SipAccount {
    SipAccount {
        id: AccountId::new(),
        name: "Test FRITZ!Box".into(),
        username: "620".into(),
        display_name: Some("Desk Phone".into()),
        domain: "fritz.box".into(),
        registrar: registrar_addr.ip().to_string(),
        registrar_port: registrar_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    }
}

#[tokio::test]
async fn test_successful_challenge_registration() {
    let mock_registrar = MockSipRegistrar::start(true, Some("secret123")).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    // Spawn client incoming packet listener
    let transport_clone = transport.clone();
    let trans_clone = transactions.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = transport_clone.recv_from(&mut buf).await
                && let Ok(text) = std::str::from_utf8(&buf[..len])
                && let Ok(SipMessage::Response(res)) = SipMessage::parse(text)
            {
                trans_clone.handle_response(res).await;
            }
        }
    });

    let account = create_test_account(mock_registrar.local_addr);
    let manager = RegistrationManager::new(account, "secret123".into(), transport, transactions);

    let reg_result = manager.register(mock_registrar.local_addr).await;
    assert!(
        reg_result.is_ok(),
        "Registration should succeed: {:?}",
        reg_result
    );

    let state = manager.get_state().await;
    assert!(matches!(state, RegistrationState::Registered { .. }));
}

#[tokio::test]
async fn test_registration_wrong_password() {
    let mock_registrar = MockSipRegistrar::start(true, Some("wrongpassword")).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let transport_clone = transport.clone();
    let trans_clone = transactions.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = transport_clone.recv_from(&mut buf).await
                && let Ok(text) = std::str::from_utf8(&buf[..len])
                && let Ok(SipMessage::Response(res)) = SipMessage::parse(text)
            {
                trans_clone.handle_response(res).await;
            }
        }
    });

    let account = create_test_account(mock_registrar.local_addr);
    let manager =
        RegistrationManager::new(account, "wrongpassword".into(), transport, transactions);

    let reg_result = manager.register(mock_registrar.local_addr).await;
    assert!(
        reg_result.is_err(),
        "Registration should fail with wrong password"
    );

    let state = manager.get_state().await;
    assert!(matches!(state, RegistrationState::Failed { .. }));
}

#[tokio::test]
async fn test_successful_unregister() {
    let mock_registrar = MockSipRegistrar::start(true, Some("secret123")).await;
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());

    let transport_clone = transport.clone();
    let trans_clone = transactions.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            if let Ok((len, _)) = transport_clone.recv_from(&mut buf).await
                && let Ok(text) = std::str::from_utf8(&buf[..len])
                && let Ok(SipMessage::Response(res)) = SipMessage::parse(text)
            {
                trans_clone.handle_response(res).await;
            }
        }
    });

    let account = create_test_account(mock_registrar.local_addr);
    let manager = RegistrationManager::new(account, "secret123".into(), transport, transactions);

    let reg_result = manager.register(mock_registrar.local_addr).await;
    assert!(reg_result.is_ok(), "Initial registration should succeed");
    assert!(matches!(
        manager.get_state().await,
        RegistrationState::Registered { .. }
    ));

    let unreg_result = manager.unregister(mock_registrar.local_addr).await;
    assert!(unreg_result.is_ok(), "Unregistration should succeed");
    assert_eq!(manager.get_state().await, RegistrationState::Unregistered);
}
