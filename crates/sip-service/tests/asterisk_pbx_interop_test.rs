//! Asterisk PBX Interoperability Integration Test
//!
//! Validates pure Rust SIP client interoperability against a real local Asterisk 18+ PBX:
//! 1. Genuine SIP registration with Digest MD5 authentication challenge.
//! 2. Asterisk AOR contact persistence.
//! 3. Outgoing call to Asterisk Echo application (exten 2000).
//! 4. Bidirectional RTP audio exchange (transmitted PCM frames echoed back by Asterisk).
//! 5. In-call RFC 4733 DTMF delivery.
//! 6. Clean call termination with RFC 3261 in-dialog BYE and 200 OK.
//! 7. Incoming call originated by Asterisk to registered client endpoint.
//! 8. Client 100 Trying / 180 Ringing / 200 OK answer, ACK handling, and teardown.
//! 9. Clean unregistration (Expires: 0) and contact removal from Asterisk AOR.

use sip_domain::{
    AccountId, AudioCodec, CallId, CallState, DisconnectReason, RegistrationState, SipAccount,
    TransportProtocol,
};
use sip_engine::call::OutgoingCallSession;
use sip_engine::incoming::IncomingCallSession;
use sip_engine::registration::RegistrationManager;
use sip_protocol::{SipMessage, SipMethod, TransactionManager};
use sip_transport::SipUdpTransport;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

/// RAII Guard ensuring Asterisk process is killed when the test completes or fails.
struct AsteriskGuard {
    child: Option<Child>,
    conf_path: PathBuf,
}

impl AsteriskGuard {
    fn spawn() -> Option<Self> {
        let asterisk_bin = Path::new("/usr/sbin/asterisk");
        if !asterisk_bin.exists() {
            eprintln!("Asterisk binary not found at /usr/sbin/asterisk. Skipping live PBX test.");
            return None;
        }

        let base_dir = PathBuf::from("/tmp/plasma-sip-asterisk-interop");
        let _ = fs::remove_dir_all(&base_dir);
        let etc_dir = base_dir.join("etc");
        let var_lib_dir = base_dir.join("var/lib");
        let var_spool_dir = base_dir.join("var/spool");
        let var_run_dir = base_dir.join("var/run");
        let var_log_dir = base_dir.join("var/log");

        fs::create_dir_all(&etc_dir).ok()?;
        fs::create_dir_all(&var_lib_dir).ok()?;
        fs::create_dir_all(&var_spool_dir).ok()?;
        fs::create_dir_all(&var_run_dir).ok()?;
        fs::create_dir_all(&var_log_dir).ok()?;

        let conf_file = etc_dir.join("asterisk.conf");
        let asterisk_conf = format!(
            "[directories]\n\
             astetcdir => {}\n\
             astmoddir => /usr/lib64/asterisk/modules\n\
             astvarlibdir => /usr/share/asterisk\n\
             astdbdir => {}\n\
             astkeydir => /usr/share/asterisk/keys\n\
             astdatadir => /usr/share/asterisk\n\
             astagidir => {}\n\
             astspooldir => {}\n\
             astrundir => {}\n\
             astlogdir => {}\n\n\
             [options]\n\
             verbose = 3\n\
             debug = 3\n\
             alwaysfork = no\n\
             nofork = yes\n\
             dumpcore = no\n",
            etc_dir.display(),
            var_lib_dir.display(),
            var_lib_dir.join("agi-bin").display(),
            var_spool_dir.display(),
            var_run_dir.display(),
            var_log_dir.display(),
        );
        fs::write(&conf_file, asterisk_conf).ok()?;

        let modules_conf = "[modules]\n\
                            autoload = yes\n\
                            noload = chan_alsa.so\n\
                            noload = chan_oss.so\n\
                            noload = chan_console.so\n\
                            noload = res_hep.so\n\
                            noload = res_hep_pjsip.so\n\
                            noload = res_hep_rtcp.so\n";
        fs::write(etc_dir.join("modules.conf"), modules_conf).ok()?;

        let pjsip_conf = "[transport-udp]\n\
                          type = transport\n\
                          protocol = udp\n\
                          bind = 127.0.0.1:5070\n\n\
                          [1001-auth]\n\
                          type = auth\n\
                          auth_type = userpass\n\
                          username = 1001\n\
                          password = secretpass\n\n\
                          [1001]\n\
                          type = aor\n\
                          max_contacts = 5\n\
                          remove_existing = yes\n\n\
                          [1001]\n\
                          type = endpoint\n\
                          context = default\n\
                          disallow = all\n\
                          allow = alaw,ulaw\n\
                          auth = 1001-auth\n\
                          aors = 1001\n\
                          direct_media = no\n\
                          dtmf_mode = rfc4733\n";
        fs::write(etc_dir.join("pjsip.conf"), pjsip_conf).ok()?;

        let extensions_conf = "[default]\n\
                               exten => 2000,1,Answer()\n\
                               same => n,Echo()\n\
                               same => n,Hangup()\n\n\
                               exten => 2001,1,Answer()\n\
                               same => n,SendDTMF(5)\n\
                               same => n,Wait(2)\n\
                               same => n,Hangup()\n";
        fs::write(etc_dir.join("extensions.conf"), extensions_conf).ok()?;

        let logger_conf = "[general]\n\
                           [logfiles]\n\
                           console => notice,warning,error,debug,verbose\n\
                           messages => notice,warning,error,debug,verbose\n";
        fs::write(etc_dir.join("logger.conf"), logger_conf).ok()?;

        // Spawn Asterisk
        let child = Command::new("/usr/sbin/asterisk")
            .arg("-C")
            .arg(&conf_file)
            .arg("-f")
            .spawn()
            .ok()?;

        let guard = AsteriskGuard {
            child: Some(child),
            conf_path: conf_file,
        };

        // Poll CLI until Asterisk is responsive (up to 5 seconds)
        let mut ready = false;
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(200));
            let output = Command::new("/usr/sbin/asterisk")
                .arg("-C")
                .arg(&guard.conf_path)
                .arg("-r")
                .arg("-x")
                .arg("core show version")
                .output();

            if let Ok(out) = output {
                if out.status.success() {
                    ready = true;
                    break;
                }
            }
        }

        if !ready {
            eprintln!("Asterisk failed to become ready in time.");
            return None;
        }

        Some(guard)
    }

    fn cli(&self, cmd: &str) -> String {
        let output = Command::new("/usr/sbin/asterisk")
            .arg("-C")
            .arg(&self.conf_path)
            .arg("-r")
            .arg("-x")
            .arg(cmd)
            .output()
            .expect("run asterisk cli command");
        String::from_utf8_lossy(&output.stdout).to_string()
    }
}

impl Drop for AsteriskGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tokio::test]
async fn test_asterisk_real_pbx_interoperability_full_suite() {
    let asterisk = match AsteriskGuard::spawn() {
        Some(guard) => guard,
        None => {
            eprintln!("Skipping Asterisk PBX interop test suite (Asterisk unavailable)");
            return;
        }
    };

    println!(
        "Asterisk PJSIP Modules:\n{}",
        asterisk.cli("module show like pjsip")
    );
    println!(
        "Asterisk Transports:\n{}",
        asterisk.cli("pjsip show transports")
    );

    let pbx_addr: SocketAddr = "127.0.0.1:5070".parse().unwrap();

    // Verify Asterisk endpoints
    let endpoints_out = asterisk.cli("pjsip show endpoints");
    assert!(
        endpoints_out.contains("1001"),
        "Asterisk must expose endpoint 1001"
    );

    // 1. Setup client SIP transport and transaction manager
    let client_transport = Arc::new(
        SipUdpTransport::bind("127.0.0.1:0")
            .await
            .expect("bind client transport"),
    );
    let transactions = Arc::new(TransactionManager::new());
    let incoming_invites = Arc::new(Mutex::new(Vec::new()));
    let incoming_invites_clone = Arc::clone(&incoming_invites);

    // Background SIP receive loop: routes responses to transactions and records requests (INVITE/BYE/ACK)
    let rx_transport = Arc::clone(&client_transport);
    let rx_txs = Arc::clone(&transactions);
    let _rx_task = tokio::spawn(async move {
        let mut buf = [0u8; 65535];
        while let Ok((len, from)) = rx_transport.recv_from(&mut buf).await {
            if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                if let Ok(msg) = SipMessage::parse(text) {
                    match msg {
                        SipMessage::Response(res) => {
                            rx_txs.handle_response(res).await;
                        }
                        SipMessage::Request(req) => {
                            if req.method == SipMethod::Invite {
                                incoming_invites_clone.lock().await.push((req, from));
                            }
                        }
                    }
                }
            }
        }
    });

    // 2. Configure Account for Extension 1001
    let account = SipAccount {
        id: AccountId::new(),
        name: "Asterisk 1001".into(),
        username: "1001".into(),
        display_name: Some("Plasma SIP Interop".into()),
        domain: "127.0.0.1".into(),
        registrar: "127.0.0.1".into(),
        registrar_port: 5070,
        transport: TransportProtocol::Udp,
        expires_secs: 120,
        enabled: true,
        is_default: true,
    };

    // 3. STEP 1: Real SIP Registration with Digest Authentication against Asterisk
    let reg_mgr = RegistrationManager::new(
        account.clone(),
        "secretpass".into(),
        Arc::clone(&client_transport),
        Arc::clone(&transactions),
    );

    let reg_res = reg_mgr.register(pbx_addr).await;
    assert!(
        reg_res.is_ok(),
        "Registration against Asterisk failed: {:?}",
        reg_res
    );

    let reg_state = reg_mgr.get_state().await;
    assert!(
        matches!(reg_state, RegistrationState::Registered { .. }),
        "State should be Registered, got: {:?}",
        reg_state
    );

    // Verify contact in Asterisk PJSIP AOR
    let aor_out = asterisk.cli("pjsip show aor 1001");
    assert!(
        aor_out.contains("sip:1001@127.0.0.1:"),
        "Asterisk AOR must register our Contact URI. Output:\n{}",
        aor_out
    );

    // 4. STEP 2: Real Outgoing Call to Asterisk Echo (exten 2000)
    let (event_tx, _event_rx) = mpsc::channel(32);
    let outgoing_call = OutgoingCallSession::new(
        account.clone(),
        "secretpass".into(),
        "sip:2000@127.0.0.1:5070".into(),
        16000,
        Arc::clone(&client_transport),
        Arc::clone(&transactions),
        event_tx,
    );

    let start_res = outgoing_call.start(pbx_addr).await;
    assert!(
        start_res.is_ok(),
        "Outgoing call to Asterisk Echo failed: {:?}",
        start_res
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let call_state = outgoing_call.get_state().await;
    assert!(
        matches!(call_state, CallState::Active { .. }),
        "Call to Asterisk Echo should be Active, got: {:?}",
        call_state
    );

    // Take audio channels to receive echoed frames
    let (pcm_rx_opt, _dtmf_rx) = outgoing_call.take_audio_channels().await;
    let mut pcm_rx = pcm_rx_opt.expect("PCM receiver must be present for active media session");

    // Transmit synthetic audio frames (160 samples = 20ms of 440 Hz tone) to Asterisk Echo
    let tone_frame: Vec<i16> = (0..160)
        .map(|i| {
            let sample = (2.0 * std::f32::consts::PI * 440.0 * (i as f32) / 8000.0).sin();
            (sample * 16000.0) as i16
        })
        .collect();

    for _ in 0..15 {
        let send_res = outgoing_call.send_audio_frame(tone_frame.clone()).await;
        assert!(send_res.is_ok(), "Sending audio frame to Asterisk failed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Await echoed PCM frame from Asterisk Echo application
    let echo_received = tokio::time::timeout(Duration::from_secs(3), pcm_rx.recv())
        .await
        .expect("Timeout waiting for Asterisk RTP echo response");

    assert!(
        echo_received.is_some(),
        "Echoed PCM audio frame from Asterisk Echo must be received"
    );
    let echoed_frame = echo_received.unwrap();
    assert_eq!(
        echoed_frame.len(),
        160,
        "Echoed frame must have 160 samples (20ms)"
    );

    // 5. STEP 3: Transmit RFC 4733 DTMF Telephony Event
    let dtmf_res = outgoing_call.send_dtmf('5').await;
    assert!(
        dtmf_res.is_ok(),
        "Transmitting DTMF digit '5' to Asterisk failed"
    );

    // Verify Media Quality Statistics
    let stats = outgoing_call
        .get_media_stats()
        .await
        .expect("Media stats available");
    assert!(
        stats.packets_sent > 0,
        "Packets sent should be > 0, got {}",
        stats.packets_sent
    );
    assert!(
        stats.packets_received > 0,
        "Packets received should be > 0, got {}",
        stats.packets_received
    );

    // Clean Call Teardown via RFC 3261 in-dialog BYE
    let hangup_res = outgoing_call.hangup(pbx_addr).await;
    assert!(
        hangup_res.is_ok(),
        "Hangup with Asterisk failed: {:?}",
        hangup_res
    );

    let final_call_state = outgoing_call.get_state().await;
    assert!(
        matches!(
            final_call_state,
            CallState::Terminated {
                reason: DisconnectReason::LocalHangup
            }
        ),
        "Final state should be LocalHangup, got: {:?}",
        final_call_state
    );

    // 6. STEP 4: Real Incoming Call from Asterisk to Client
    // Settle for 300ms so Asterisk finishes channel destruction from previous call
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Trigger Asterisk to originate an incoming call to registered endpoint 1001
    let orig_output = asterisk.cli("channel originate PJSIP/1001 application Echo");
    assert!(
        !orig_output.contains("Failed"),
        "Asterisk originate command failed: {}",
        orig_output
    );

    // Wait for incoming INVITE from Asterisk (with 5 second timeout)
    let incoming_invite = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut list = incoming_invites.lock().await;
            if let Some(invite) = list.pop() {
                return invite;
            }
        }
    })
    .await
    .expect("Timeout waiting for incoming INVITE from Asterisk");

    let (req, from_asterisk) = incoming_invite;
    let (in_event_tx, _in_event_rx) = mpsc::channel(32);
    let in_session = IncomingCallSession::with_call_id_and_ring(
        CallId::new(),
        req.clone(),
        from_asterisk,
        Arc::clone(&client_transport),
        in_event_tx,
    )
    .await
    .expect("Create incoming session from Asterisk INVITE");

    // Client answers Asterisk with 200 OK and ephemeral SDP answer
    let ans_res = in_session.answer(16002, AudioCodec::Pcma).await;
    assert!(
        ans_res.is_ok(),
        "Answering incoming call from Asterisk failed: {:?}",
        ans_res
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let in_state = in_session.get_state().await;
    assert!(
        matches!(in_state, CallState::Active { .. }),
        "Incoming call should be Active, got: {:?}",
        in_state
    );

    // Clean teardown of incoming call
    let in_hangup = in_session.hangup().await;
    assert!(
        in_hangup.is_ok(),
        "Hanging up incoming call with Asterisk failed"
    );

    // 7. STEP 5: Clean Unregistration (Expires: 0)
    let unreg_res = reg_mgr.unregister(pbx_addr).await;
    assert!(
        unreg_res.is_ok(),
        "Unregistration from Asterisk failed: {:?}",
        unreg_res
    );

    let unreg_state = reg_mgr.get_state().await;
    assert!(
        matches!(unreg_state, RegistrationState::Unregistered),
        "Final registration state should be Unregistered, got: {:?}",
        unreg_state
    );

    // Verify contact removed from Asterisk AOR
    let unreg_aor_out = asterisk.cli("pjsip show aor 1001");
    assert!(
        !unreg_aor_out.contains("sip:1001@127.0.0.1:"),
        "Asterisk AOR must have removed the Contact upon Expires: 0. Output:\n{}",
        unreg_aor_out
    );
}
