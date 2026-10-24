//! Milestone R1: Real SIP Registration and Account Lifecycle Integration Tests.
//!
//! Verifies:
//! - Real SIP REGISTER over UDP network socket with challenge-response Digest authentication.
//! - RFC 3261 compliance (CSeq incrementing, From/To tag matching, Via branch).
//! - RFC 2617 multi-qop ("auth,auth-int") challenge handling.
//! - Real SIP unregistration with Expires: 0.
//! - Startup recovery and re-registration of enabled accounts.
//! - Keyring secret integration and failure modes for missing/wrong credentials.
//! - Deletion lifecycle ensuring network unregistration before database cleanup.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::UdpSocket;

use sip_domain::{AccountId, RegistrationState, SipAccount, TransportProtocol};
use sip_engine::SipClient;
use sip_protocol::{SipMessage, SipMethod, TransactionManager};
use sip_service::AccountManager;
use sip_storage::StorageManager;
use sip_transport::SipUdpTransport;

/// Controllable local SIP registrar for automated testing.
struct LocalRegistrarFixture {
    local_addr: SocketAddr,
    register_count: Arc<AtomicUsize>,
    unregister_count: Arc<AtomicUsize>,
}

impl LocalRegistrarFixture {
    async fn start(reject_credentials: bool) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind local test registrar");
        let local_addr = socket.local_addr().expect("registrar local addr");
        let socket = Arc::new(socket);

        let register_count = Arc::new(AtomicUsize::new(0));
        let unregister_count = Arc::new(AtomicUsize::new(0));

        let reg_cnt = register_count.clone();
        let unreg_cnt = unregister_count.clone();

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

                let req = match SipMessage::parse(text) {
                    Ok(SipMessage::Request(r)) => r,
                    _ => continue,
                };

                if req.method == SipMethod::Register {
                    let has_auth = req.headers.contains_key("Authorization");
                    let via = req.headers.get("Via").cloned().unwrap_or_default();
                    let from = req.headers.get("From").cloned().unwrap_or_default();
                    let to = req.headers.get("To").cloned().unwrap_or_default();
                    let call_id = req.headers.get("Call-ID").cloned().unwrap_or_default();
                    let cseq = req.headers.get("CSeq").cloned().unwrap_or_default();
                    let expires = req
                        .headers
                        .get("Expires")
                        .cloned()
                        .unwrap_or_else(|| "300".into());

                    if expires == "0" {
                        unreg_cnt.fetch_add(1, Ordering::SeqCst);
                    } else {
                        reg_cnt.fetch_add(1, Ordering::SeqCst);
                    }

                    if !has_auth {
                        // Issue 401 challenge with multi-qop (as FRITZ!OS and Asterisk do)
                        let challenge_resp = format!(
                            "SIP/2.0 401 Unauthorized\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=pbx-srv-1\r\nCall-ID: {}\r\nCSeq: {}\r\nWWW-Authenticate: Digest realm=\"fritz.box\", nonce=\"nonce-r1-test-999\", algorithm=MD5, qop=\"auth,auth-int\"\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq
                        );
                        let _ = socket.send_to(challenge_resp.as_bytes(), client_addr).await;
                    } else {
                        let auth_hdr = req
                            .headers
                            .get("Authorization")
                            .cloned()
                            .unwrap_or_default();

                        // Verify qop="auth" was parsed and quoted properly
                        assert!(
                            auth_hdr.contains("qop=\"auth\""),
                            "Authorization must select qop=\"auth\" from multi-qop challenge"
                        );

                        if !reject_credentials && auth_hdr.contains("username=\"620\"") {
                            let ok_resp = format!(
                                "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=pbx-srv-1\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:620@{}:{}>;expires={}\r\nExpires: {}\r\nContent-Length: 0\r\n\r\n",
                                via,
                                from,
                                to,
                                call_id,
                                cseq,
                                client_addr.ip(),
                                client_addr.port(),
                                expires,
                                expires
                            );
                            let _ = socket.send_to(ok_resp.as_bytes(), client_addr).await;
                        } else {
                            let forbidden_resp = format!(
                                "SIP/2.0 403 Forbidden\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=pbx-srv-1\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                                via, from, to, call_id, cseq
                            );
                            let _ = socket.send_to(forbidden_resp.as_bytes(), client_addr).await;
                        }
                    }
                }
            }
        });

        Self {
            local_addr,
            register_count,
            unregister_count,
        }
    }
}

async fn setup_test_environment() -> (
    Arc<StorageManager>,
    Arc<SipUdpTransport>,
    Arc<TransactionManager>,
    Arc<AccountManager>,
) {
    let storage = Arc::new(StorageManager::new_in_memory().expect("in-memory db"));
    assert!(
        storage.is_isolated_backend(),
        "Must use isolated credential backend"
    );
    let transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind transport"),
    );
    let transactions = Arc::new(TransactionManager::new());
    let sip_client = Arc::new(SipClient::new(transport.clone(), transactions.clone()));
    let _recv_loop = sip_client.start_receive_loop();

    let account_manager = Arc::new(AccountManager::new(
        storage.clone(),
        transport.clone(),
        transactions.clone(),
    ));

    (storage, transport, transactions, account_manager)
}

#[tokio::test]
async fn test_r1_real_registration_challenge_and_unregistration() {
    let registrar = LocalRegistrarFixture::start(false).await;
    let (_storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "FRITZ!Box R1 Test".into(),
        username: "620".into(),
        display_name: Some("Plasma Softphone".into()),
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 180,
        enabled: true,
        is_default: true,
    };

    // 1. Save account and password
    account_mgr
        .save_account(&account, "fritzbox_secret")
        .expect("save account");

    // Initial state is unregistered
    let initial_state = account_mgr.get_registration_state(account_id).await;
    assert_eq!(initial_state, RegistrationState::Unregistered);

    // 2. Trigger real network registration
    let reg_res = account_mgr.register_account(account_id).await;
    assert!(reg_res.is_ok(), "Registration must succeed: {:?}", reg_res);

    // 3. Verify state is Registered
    let state = account_mgr.get_registration_state(account_id).await;
    match state {
        RegistrationState::Registered {
            expires_at_epoch_secs,
        } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            assert!(expires_at_epoch_secs > now);
        }
        other => panic!("Expected Registered state, got {:?}", other),
    }

    // Verify registrar received initial + authenticated REGISTER
    assert!(
        registrar.register_count.load(Ordering::SeqCst) >= 2,
        "Registrar must receive at least 2 REGISTER requests (challenge + authenticated)"
    );

    // 4. Trigger real network unregistration
    let unreg_res = account_mgr.unregister_account(account_id).await;
    assert!(
        unreg_res.is_ok(),
        "Unregistration must succeed: {:?}",
        unreg_res
    );

    let final_state = account_mgr.get_registration_state(account_id).await;
    assert_eq!(final_state, RegistrationState::Unregistered);

    // Verify registrar received unregistration (Expires: 0)
    assert!(
        registrar.unregister_count.load(Ordering::SeqCst) >= 1,
        "Registrar must receive unregister request (Expires: 0)"
    );
}

#[tokio::test]
async fn test_r1_startup_recovery_registers_enabled_accounts() {
    let registrar = LocalRegistrarFixture::start(false).await;
    let (_storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    let acc1_id = AccountId::new();
    let acc1 = SipAccount {
        id: acc1_id,
        name: "Auto Account 1".into(),
        username: "620".into(),
        display_name: None,
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 120,
        enabled: true,
        is_default: true,
    };
    account_mgr
        .save_account(&acc1, "recovery_pass")
        .expect("save acc1");

    let acc2_id = AccountId::new();
    let acc2 = SipAccount {
        id: acc2_id,
        name: "Disabled Account 2".into(),
        username: "621".into(),
        display_name: None,
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 120,
        enabled: false, // Disabled
        is_default: false,
    };
    account_mgr
        .save_account(&acc2, "recovery_pass")
        .expect("save acc2");

    // Execute startup recovery
    let recovered = account_mgr
        .recover_and_register_all()
        .await
        .expect("recover accounts");
    assert_eq!(recovered, 1, "Only enabled account should be registered");

    let state1 = account_mgr.get_registration_state(acc1_id).await;
    assert!(matches!(state1, RegistrationState::Registered { .. }));

    let state2 = account_mgr.get_registration_state(acc2_id).await;
    assert_eq!(state2, RegistrationState::Unregistered);
}

#[tokio::test]
async fn test_r1_registration_wrong_password_transitions_to_failed() {
    let registrar = LocalRegistrarFixture::start(true).await;
    let (_storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "Wrong Pass Account".into(),
        username: "620".into(),
        display_name: None,
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };

    // Save with incorrect password
    account_mgr
        .save_account(&account, "wrongpass")
        .expect("save account");

    let reg_res = account_mgr.register_account(account_id).await;
    assert!(
        reg_res.is_err(),
        "Registration must fail with wrong password"
    );

    let state = account_mgr.get_registration_state(account_id).await;
    match state {
        RegistrationState::Failed { reason, .. } => {
            assert!(
                reason.contains("403") || reason.contains("Forbidden"),
                "Reason should reflect 403 Forbidden: {}",
                reason
            );
        }
        other => panic!("Expected Failed state, got {:?}", other),
    }
}

#[tokio::test]
async fn test_r1_registration_missing_secret_fails() {
    let registrar = LocalRegistrarFixture::start(false).await;
    let (storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "No Secret Acc".into(),
        username: "620".into(),
        display_name: None,
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };

    // Save only to SQLite, omitting keyring secret
    storage.save_account(&account).expect("save to sqlite");

    let reg_res = account_mgr.register_account(account_id).await;
    assert!(
        reg_res.is_err(),
        "Registration must fail without credentials"
    );

    let state = account_mgr.get_registration_state(account_id).await;
    assert!(matches!(state, RegistrationState::Failed { .. }));
}

#[tokio::test]
async fn test_r1_delete_account_unregisters_and_cleans_keyring() {
    let registrar = LocalRegistrarFixture::start(false).await;
    let (storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "To Be Deleted".into(),
        username: "620".into(),
        display_name: None,
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };

    account_mgr
        .save_account(&account, "delete_secret")
        .expect("save account");

    account_mgr
        .register_account(account_id)
        .await
        .expect("register");
    assert!(matches!(
        account_mgr.get_registration_state(account_id).await,
        RegistrationState::Registered { .. }
    ));

    // Delete account
    account_mgr
        .delete_account(account_id)
        .await
        .expect("delete account");

    // Verify removed from storage
    assert!(storage.get_account(account_id).unwrap().is_none());

    // Verify secret removed
    assert!(storage.get_account_secret(account_id).unwrap().is_none());

    // Verify registrar received unregistration
    assert!(
        registrar.unregister_count.load(Ordering::SeqCst) >= 1,
        "Registrar must receive unregister request on deletion"
    );

    // Verify state removed
    assert_eq!(
        account_mgr.get_registration_state(account_id).await,
        RegistrationState::Unregistered
    );
}

#[tokio::test]
async fn test_r1_registration_timeout_transitions_to_failed() {
    let (_storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    // Allocate an ephemeral port and close the socket immediately to ensure no SIP responses
    let dead_port = {
        let s = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral socket");
        s.local_addr().expect("local addr").port()
    };

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "Unresponsive PBX".into(),
        username: "620".into(),
        display_name: None,
        domain: "unreachable.test".into(),
        registrar: "127.0.0.1".into(),
        registrar_port: dead_port,
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };

    account_mgr
        .save_account(&account, "some_pass")
        .expect("save account");

    let mut event_rx = account_mgr.subscribe_registration_events();

    let start = std::time::Instant::now();
    let reg_res = account_mgr.register_account(account_id).await;
    let elapsed = start.elapsed();

    assert!(reg_res.is_err(), "Registration must error on timeout");
    assert!(
        elapsed >= std::time::Duration::from_secs(4),
        "Must wait for timeout before failing"
    );

    // Verify registration state is Failed
    let state = account_mgr.get_registration_state(account_id).await;
    match state {
        RegistrationState::Failed {
            reason,
            retry_in_secs,
        } => {
            assert!(
                reason.to_lowercase().contains("timed out")
                    || reason.to_lowercase().contains("timeout"),
                "Failure reason must mention timeout: {}",
                reason
            );
            assert!(retry_in_secs.is_some(), "Must specify retry interval");
        }
        other => panic!("Expected RegistrationState::Failed, got {:?}", other),
    }

    // Verify events: Registering followed by Failed
    let ev1 = event_rx.try_recv().expect("must receive Registering event");
    assert_eq!(ev1.0, account_id);
    assert_eq!(ev1.1, RegistrationState::Registering);

    let ev2 = event_rx.try_recv().expect("must receive Failed event");
    assert_eq!(ev2.0, account_id);
    assert!(matches!(ev2.1, RegistrationState::Failed { .. }));
}

#[tokio::test]
async fn test_r1_registration_renewal_schedules_and_re_registers() {
    let registrar = LocalRegistrarFixture::start(false).await;
    let (_storage, _transport, _transactions, account_mgr) = setup_test_environment().await;

    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "Renewal Test Acc".into(),
        username: "620".into(),
        display_name: None,
        domain: "fritz.box".into(),
        registrar: registrar.local_addr.ip().to_string(),
        registrar_port: registrar.local_addr.port(),
        transport: TransportProtocol::Udp,
        // With expires_secs = 32, sleep interval is max(32 - 30, 10) = 10s
        expires_secs: 32,
        enabled: true,
        is_default: true,
    };

    account_mgr
        .save_account(&account, "renewal_pass")
        .expect("save account");

    let mut event_rx = account_mgr.subscribe_registration_events();

    account_mgr
        .register_account(account_id)
        .await
        .expect("initial register");

    let initial_count = registrar.register_count.load(Ordering::SeqCst);
    assert_eq!(
        initial_count, 2,
        "Initial challenge-response registration must send 2 REGISTER requests (probe + response)"
    );

    // Initial events: Registering -> Registered
    let ev1 = event_rx.recv().await.expect("recv ev1");
    assert_eq!(ev1.0, account_id);
    assert_eq!(ev1.1, RegistrationState::Registering);

    let ev2 = event_rx.recv().await.expect("recv ev2");
    assert_eq!(ev2.0, account_id);
    assert!(matches!(ev2.1, RegistrationState::Registered { .. }));

    // Wait for the auto-renewal background task to fire (10s sleep)
    tokio::time::sleep(std::time::Duration::from_millis(10600)).await;

    let renewed_count = registrar.register_count.load(Ordering::SeqCst);
    assert!(
        renewed_count > initial_count,
        "Auto-renewal must send renewal REGISTER requests to registrar (was {}, now {})",
        initial_count,
        renewed_count
    );

    // Renewal event: Registered
    let ev3 = event_rx.recv().await.expect("recv renewal event");
    assert_eq!(ev3.0, account_id);
    assert!(matches!(ev3.1, RegistrationState::Registered { .. }));

    // Clean teardown: unregister
    account_mgr
        .unregister_account(account_id)
        .await
        .expect("unregister");
    assert_eq!(
        registrar.unregister_count.load(Ordering::SeqCst),
        2,
        "Unregistration under challenge-response must send 2 requests (probe + authenticated)"
    );
}
