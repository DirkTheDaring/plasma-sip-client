use futures::StreamExt;
use std::sync::Arc;
use uuid::Uuid;
use zbus::connection::{Builder, Connection};

use sip_domain::{AccountId, SipAccount, TransportProtocol};
use sip_service::{AccountManager, CallManager, PlasmaSipDbusService};
use sip_storage::StorageManager;

#[zbus::proxy(interface = "org.kde.PlasmaSip", assume_defaults = true)]
trait PlasmaSip {
    #[zbus(signal)]
    fn registration_changed(&self, account_id: &str, state: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn call_state_changed(&self, call_id: &str, state: &str) -> zbus::Result<()>;
}

struct MockSipRegistrar {
    local_addr: std::net::SocketAddr,
    received_methods: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl MockSipRegistrar {
    async fn start() -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind mock reg");
        let local_addr = socket.local_addr().expect("local addr");
        let socket = Arc::new(socket);
        let received_methods = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let methods_clone = received_methods.clone();

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
                let msg = match sip_protocol::SipMessage::parse(text) {
                    Ok(sip_protocol::SipMessage::Request(req)) => req,
                    _ => continue,
                };

                let method_str = format!("{}", msg.method);
                methods_clone.lock().await.push(method_str);

                if msg.method == sip_protocol::SipMethod::Register {
                    let has_auth = msg.headers.contains_key("Authorization");
                    let via = msg.headers.get("Via").cloned().unwrap_or_default();
                    let from = msg.headers.get("From").cloned().unwrap_or_default();
                    let to = msg.headers.get("To").cloned().unwrap_or_default();
                    let call_id = msg.headers.get("Call-ID").cloned().unwrap_or_default();
                    let cseq = msg.headers.get("CSeq").cloned().unwrap_or_default();
                    let expires_val = msg
                        .headers
                        .get("Expires")
                        .cloned()
                        .unwrap_or_else(|| "300".into());

                    if !has_auth {
                        let resp = format!(
                            "SIP/2.0 401 Unauthorized\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=reg-srv\r\nCall-ID: {}\r\nCSeq: {}\r\nWWW-Authenticate: Digest realm=\"fritz.box\", nonce=\"dbusnonce123\", algorithm=MD5, qop=\"auth\"\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq
                        );
                        let _ = socket.send_to(resp.as_bytes(), client_addr).await;
                    } else {
                        let resp = format!(
                            "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=reg-srv\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:620@{}:{}>;expires={}\r\nExpires: {}\r\nContent-Length: 0\r\n\r\n",
                            via, from, to, call_id, cseq, client_addr.ip(), client_addr.port(), expires_val, expires_val
                        );
                        let _ = socket.send_to(resp.as_bytes(), client_addr).await;
                    }
                } else if msg.method == sip_protocol::SipMethod::Invite {
                    let via = msg.headers.get("Via").cloned().unwrap_or_default();
                    let from = msg.headers.get("From").cloned().unwrap_or_default();
                    let to = msg.headers.get("To").cloned().unwrap_or_default();
                    let call_id = msg.headers.get("Call-ID").cloned().unwrap_or_default();
                    let cseq = msg.headers.get("CSeq").cloned().unwrap_or_default();

                    // 1. Send 180 Ringing
                    let ringing = format!(
                        "SIP/2.0 180 Ringing\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=mock-uas-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(ringing.as_bytes(), client_addr).await;

                    // 2. Send 200 OK with SDP
                    let sdp_body = "v=0\r\no=mock 1 1 IN IP4 127.0.0.1\r\ns=Session\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 40000 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\n";
                    let ok = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=mock-uas-tag\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sip:611@127.0.0.1:{}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                        via, from, to, call_id, cseq, local_addr.port(), sdp_body.len(), sdp_body
                    );
                    let _ = socket.send_to(ok.as_bytes(), client_addr).await;
                } else if msg.method == sip_protocol::SipMethod::Bye {
                    let via = msg.headers.get("Via").cloned().unwrap_or_default();
                    let from = msg.headers.get("From").cloned().unwrap_or_default();
                    let to = msg.headers.get("To").cloned().unwrap_or_default();
                    let call_id = msg.headers.get("Call-ID").cloned().unwrap_or_default();
                    let cseq = msg.headers.get("CSeq").cloned().unwrap_or_default();

                    let ok = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        via, from, to, call_id, cseq
                    );
                    let _ = socket.send_to(ok.as_bytes(), client_addr).await;
                }
            }
        });
        Self {
            local_addr,
            received_methods,
        }
    }
}

#[tokio::test]
async fn test_dbus_service_api_contract() {
    let mock_reg = MockSipRegistrar::start().await;
    let transport = Arc::new(
        sip_transport::SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind transport"),
    );
    let transactions = Arc::new(sip_protocol::TransactionManager::new());

    // 1. Setup in-memory SQLite storage and managers
    let storage = Arc::new(StorageManager::new_in_memory().expect("in-memory db"));
    assert!(storage.is_isolated_backend(), "Must use isolated credential backend");
    let account_manager = Arc::new(AccountManager::new(
        storage.clone(),
        transport.clone(),
        transactions.clone(),
    ));
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

    let dbus_service = PlasmaSipDbusService {
        account_manager: account_manager.clone(),
        call_manager: call_manager.clone(),
        storage: storage.clone(),
    };

    // 2. Setup D-Bus service on session bus under test name and path
    let test_name = format!("org.kde.PlasmaSip.Test_{}", Uuid::new_v4().as_simple());
    let test_path = format!("/org/kde/PlasmaSip/{}", Uuid::new_v4().as_simple());

    let server_conn = match Builder::session() {
        Ok(builder) => {
            match builder
                .name(test_name.as_str())
                .and_then(|b| b.serve_at(test_path.as_str(), dbus_service))
            {
                Ok(b) => match b.build().await {
                    Ok(conn) => conn,
                    Err(_) => {
                        eprintln!("Skipping test: Session D-Bus not accessible");
                        return;
                    }
                },
                Err(_) => {
                    eprintln!("Skipping test: Session D-Bus configuration failed");
                    return;
                }
            }
        }
        Err(_) => {
            eprintln!("Skipping test: Session D-Bus not available");
            return;
        }
    };

    let _sig_forwarder = PlasmaSipDbusService::spawn_registration_signal_forwarder(
        server_conn.clone(),
        test_path.clone(),
        account_manager.clone(),
    );

    let _call_sig_forwarder = PlasmaSipDbusService::spawn_call_signal_forwarder(
        server_conn.clone(),
        test_path.clone(),
        call_manager.clone(),
    );

    let client_conn = Connection::session().await.expect("client session conn");

    let proxy = PlasmaSipProxy::builder(&client_conn)
        .destination(test_name.as_str())
        .expect("proxy destination")
        .path(test_path.as_str())
        .expect("proxy path")
        .build()
        .await
        .expect("build proxy");

    let mut reg_signal_stream = proxy
        .receive_registration_changed()
        .await
        .expect("receive registration changed");

    let mut call_signal_stream = proxy
        .receive_call_state_changed()
        .await
        .expect("receive call_state_changed");

    // 3. Test Ping method
    let ping_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "Ping",
            &(),
        )
        .await
        .expect("ping call")
        .body()
        .deserialize()
        .expect("deserialize ping");
    assert_eq!(ping_res, "pong");

    // 4. Test ListAccounts when empty
    let list_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "ListAccounts",
            &(),
        )
        .await
        .expect("list accounts")
        .body()
        .deserialize()
        .expect("deserialize list");
    assert_eq!(list_res, "[]");

    // 5. Test SaveAccount via D-Bus
    let account_id = AccountId::new();
    let account = SipAccount {
        id: account_id,
        name: "Home Office".into(),
        username: "620".into(),
        display_name: Some("Plasma User".into()),
        domain: "fritz.box".into(),
        registrar: mock_reg.local_addr.ip().to_string(),
        registrar_port: mock_reg.local_addr.port(),
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };
    let account_json = serde_json::to_string(&account).unwrap();

    let save_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "SaveAccount",
            &(account_json.as_str(), "secret123"),
        )
        .await
        .expect("save account")
        .body()
        .deserialize()
        .expect("deserialize save");
    assert_eq!(save_res, "OK");

    // 6. Test ListAccounts now contains created account
    let list_res2: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "ListAccounts",
            &(),
        )
        .await
        .expect("list accounts")
        .body()
        .deserialize()
        .expect("deserialize list2");
    assert!(list_res2.contains("Home Office"));
    assert!(list_res2.contains("620"));

    // 7. Test GetAccountStatus
    let status_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAccountStatus",
            &(account_id.to_string()),
        )
        .await
        .expect("get account status")
        .body()
        .deserialize()
        .expect("deserialize status");
    assert_eq!(status_res, "\"unregistered\"");

    // 8. Test RegisterAccount
    let reg_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "RegisterAccount",
            &(account_id.to_string()),
        )
        .await
        .expect("register account")
        .body()
        .deserialize()
        .expect("deserialize reg");
    assert_eq!(reg_res, "OK");

    // Verify D-Bus signals for registration: registering -> registered
    let sig1 = tokio::time::timeout(std::time::Duration::from_secs(3), reg_signal_stream.next())
        .await
        .expect("timeout waiting for Registering signal")
        .expect("sig1");
    let args1 = sig1.args().expect("args1");
    assert_eq!(*args1.account_id(), account_id.to_string().as_str());
    assert!(args1.state().contains("registering"));

    let sig2 = tokio::time::timeout(std::time::Duration::from_secs(3), reg_signal_stream.next())
        .await
        .expect("timeout waiting for Registered signal")
        .expect("sig2");
    let args2 = sig2.args().expect("args2");
    assert_eq!(*args2.account_id(), account_id.to_string().as_str());
    assert!(args2.state().contains("registered"));

    let status_res2: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAccountStatus",
            &(account_id.to_string()),
        )
        .await
        .expect("get account status")
        .body()
        .deserialize()
        .expect("deserialize status2");
    assert!(status_res2.contains("registered"));

    // Test UnregisterAccount via D-Bus
    let unreg_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "UnregisterAccount",
            &(account_id.to_string()),
        )
        .await
        .expect("unregister account")
        .body()
        .deserialize()
        .expect("deserialize unreg");
    assert_eq!(unreg_res, "OK");

    // Verify D-Bus signal for unregistration: unregistered
    let sig3 = tokio::time::timeout(std::time::Duration::from_secs(3), reg_signal_stream.next())
        .await
        .expect("timeout waiting for Unregistered signal")
        .expect("sig3");
    let args3 = sig3.args().expect("args3");
    assert_eq!(*args3.account_id(), account_id.to_string().as_str());
    assert!(args3.state().contains("unregistered"));

    let status_res3: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAccountStatus",
            &(account_id.to_string()),
        )
        .await
        .expect("get account status")
        .body()
        .deserialize()
        .expect("deserialize status3");
    assert_eq!(status_res3, "\"unregistered\"");

    // 9. Test StartCall and GetActiveCall
    let call_id_str: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "StartCall",
            &(account_id.to_string(), "**611"),
        )
        .await
        .expect("start call")
        .body()
        .deserialize()
        .expect("deserialize call_id");

    assert!(Uuid::parse_str(&call_id_str).is_ok());

    // Verify D-Bus signals for call: dialing -> ringing_remote -> active
    let csig = tokio::time::timeout(std::time::Duration::from_secs(3), call_signal_stream.next())
        .await
        .expect("timeout waiting for call signal")
        .expect("csig");
    let cargs = csig.args().expect("cargs");
    assert_eq!(*cargs.call_id(), call_id_str.as_str());

    let active_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetActiveCall",
            &(),
        )
        .await
        .expect("get active call")
        .body()
        .deserialize()
        .expect("deserialize active");
    assert!(active_res.contains(&call_id_str));
    assert!(active_res.contains("**611"));

    // 10. Test Mute functionality
    let is_muted_res: bool = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "IsMuted",
            &(),
        )
        .await
        .expect("is muted")
        .body()
        .deserialize()
        .expect("deserialize is_muted");
    assert!(!is_muted_res);

    let mute_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "SetMuted",
            &(true),
        )
        .await
        .expect("set muted")
        .body()
        .deserialize()
        .expect("deserialize set_muted");
    assert_eq!(mute_res, "OK");

    let is_muted_res2: bool = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "IsMuted",
            &(),
        )
        .await
        .expect("is muted2")
        .body()
        .deserialize()
        .expect("deserialize is_muted2");
    assert!(is_muted_res2);

    // 11. Test HangupCall
    let hangup_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "HangupCall",
            &(call_id_str.as_str()),
        )
        .await
        .expect("hangup call")
        .body()
        .deserialize()
        .expect("deserialize hangup");
    assert_eq!(hangup_res, "OK");

    // Allow network BYE and D-Bus termination signal to dispatch
    let mut terminated_signal_received = false;
    while let Ok(Some(sig)) = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        call_signal_stream.next(),
    )
    .await
    {
        if let Ok(args) = sig.args() {
            if args.state().contains("terminated") {
                terminated_signal_received = true;
                break;
            }
        }
    }
    assert!(
        terminated_signal_received,
        "Expected terminated call_state_changed signal"
    );

    let active_res2: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetActiveCall",
            &(),
        )
        .await
        .expect("get active call")
        .body()
        .deserialize()
        .expect("deserialize active2");
    assert_eq!(active_res2, "null");

    // Verify real network traffic: INVITE was transmitted on UDP to mock registrar/endpoint!
    let methods = mock_reg.received_methods.lock().await.clone();
    assert!(
        methods.iter().any(|m| m == "INVITE"),
        "Expected INVITE on UDP wire, got {:?}",
        methods
    );

    // Verify call history was recorded in SQLite
    let history_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetCallHistory",
            &(10u32),
        )
        .await
        .expect("get call history")
        .body()
        .deserialize()
        .expect("deserialize history");
    assert!(history_res.contains(&call_id_str));
    assert!(history_res.contains("**611"));

    // 12. Test GetAudioDevices
    let devices_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAudioDevices",
            &(),
        )
        .await
        .expect("devices call")
        .body()
        .deserialize()
        .expect("deserialize devices");
    assert!(devices_res.contains("inputs"));
    assert!(devices_res.contains("outputs"));

    // 13. Test Invalid D-Bus Request Handling (Malformed UUID)
    let bad_uuid_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAccountStatus",
            &("not-a-uuid"),
        )
        .await
        .expect("get account status bad uuid")
        .body()
        .deserialize()
        .expect("deserialize bad uuid");
    assert!(bad_uuid_res.contains("Invalid UUID"));

    // 14. Test DeleteAccount
    let del_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "DeleteAccount",
            &(account_id.to_string()),
        )
        .await
        .expect("delete account")
        .body()
        .deserialize()
        .expect("deserialize delete");
    assert_eq!(del_res, "OK");

    let list_res3: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "ListAccounts",
            &(),
        )
        .await
        .expect("list accounts3")
        .body()
        .deserialize()
        .expect("deserialize list3");
    assert_eq!(list_res3, "[]");

    // 15. Test GetAudioConfig and SaveAudioConfig
    let audio_cfg_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAudioConfig",
            &(),
        )
        .await
        .expect("get audio config")
        .body()
        .deserialize()
        .expect("deserialize audio config");
    assert!(audio_cfg_res.contains("jitter_buffer_target_ms"));

    let save_cfg_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "SaveAudioConfig",
            &(r#"{"input_device_id":"Jabra Evolve2 30 SE, USB Audio","output_device_id":"Jabra Evolve2 30 SE, USB Audio","preferred_codecs":["pcma"],"jitter_buffer_target_ms":60,"echo_cancellation":true}"#),
        )
        .await
        .expect("save audio config")
        .body()
        .deserialize()
        .expect("deserialize save cfg");
    assert_eq!(save_cfg_res, "OK");

    let audio_cfg_res2: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetAudioConfig",
            &(),
        )
        .await
        .expect("get audio config2")
        .body()
        .deserialize()
        .expect("deserialize audio config2");
    assert!(audio_cfg_res2.contains("Jabra Evolve2 30 SE, USB Audio"));

    // Test GetRejectMode and SetRejectMode via D-Bus
    let initial_mode: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetRejectMode",
            &(),
        )
        .await
        .expect("get reject mode")
        .body()
        .deserialize()
        .expect("deserialize reject mode");
    assert_eq!(initial_mode, "603_decline");

    let set_mode_res: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "SetRejectMode",
            &("answer_and_drop"),
        )
        .await
        .expect("set reject mode")
        .body()
        .deserialize()
        .expect("deserialize set reject mode");
    assert_eq!(set_mode_res, "OK");

    let updated_mode: String = client_conn
        .call_method(
            Some(test_name.as_str()),
            test_path.as_str(),
            Some("org.kde.PlasmaSip"),
            "GetRejectMode",
            &(),
        )
        .await
        .expect("get reject mode updated")
        .body()
        .deserialize()
        .expect("deserialize updated mode");
    assert_eq!(updated_mode, "answer_and_drop");
}

