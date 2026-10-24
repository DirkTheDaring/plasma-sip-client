//! CXX-Qt D-Bus Bridge connecting KDE Plasma 6 Kirigami QML interface
//! to the background SIP daemon (`plasma-sipd`) over Session D-Bus (`org.kde.PlasmaSip`).

#![allow(clippy::too_many_arguments, clippy::collapsible_if)]

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use futures::StreamExt;
use std::pin::Pin;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use zbus::Connection;

#[zbus::proxy(
    default_service = "org.kde.PlasmaSip",
    default_path = "/org/kde/PlasmaSip",
    interface = "org.kde.PlasmaSip"
)]
pub trait PlasmaSipDaemon {
    fn ping(&self) -> zbus::Result<String>;
    fn start_call(&self, account_id: &str, target: &str) -> zbus::Result<String>;
    fn answer_call(&self, call_id: &str) -> zbus::Result<String>;
    fn reject_call(&self, call_id: &str) -> zbus::Result<String>;
    fn hangup_call(&self, call_id: &str) -> zbus::Result<String>;
    fn hold_call(&self, call_id: &str) -> zbus::Result<String>;
    fn resume_call(&self, call_id: &str) -> zbus::Result<String>;
    fn send_dtmf(&self, call_id: &str, digit: &str) -> zbus::Result<String>;
    fn set_muted(&self, muted: bool) -> zbus::Result<String>;
    fn is_muted(&self) -> zbus::Result<bool>;
    fn get_active_call(&self) -> zbus::Result<String>;
    fn list_accounts(&self) -> zbus::Result<String>;
    fn get_call_history(&self, limit: u32) -> zbus::Result<String>;
    fn get_audio_devices(&self) -> zbus::Result<String>;
    fn get_audio_config(&self) -> zbus::Result<String>;
    fn save_audio_config(&self, config_json: &str) -> zbus::Result<String>;
    fn test_ringtone(&self, ringtone: &str, custom_path: &str, volume: u32)
    -> zbus::Result<String>;
    fn stop_test_ringtone(&self) -> zbus::Result<String>;
    fn get_account_status(&self, account_id: &str) -> zbus::Result<String>;
    fn register_account(&self, account_id: &str) -> zbus::Result<String>;
    fn reconnect_all(&self) -> zbus::Result<String>;
    fn save_account(&self, config_json: &str, password: &str) -> zbus::Result<String>;
    fn delete_account(&self, account_id: &str) -> zbus::Result<String>;
    fn get_reject_mode(&self) -> zbus::Result<String>;
    fn set_reject_mode(&self, mode: &str) -> zbus::Result<String>;
    fn clear_call_history(&self) -> zbus::Result<bool>;
    fn delete_call_log_entry(&self, id: &str) -> zbus::Result<bool>;
    fn list_contacts(&self) -> zbus::Result<String>;
    fn search_contacts(&self, query: &str) -> zbus::Result<String>;
    fn resolve_caller_name(&self, number: &str) -> zbus::Result<String>;
    fn sync_phonebook(&self) -> zbus::Result<String>;

    #[zbus(signal)]
    fn registration_changed(&self, account_id: String, state: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn incoming_call(&self, call_id: String, caller: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn call_state_changed(&self, call_id: String, state: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn raise_window(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn contacts_changed(&self) -> zbus::Result<()>;
}

pub enum UiCommand {
    Dial(String),
    Answer,
    Reject,
    SetRejectMode(String),
    Hangup,
    Hold,
    Resume,
    ToggleMute,
    SendDtmf(char),
    RefreshAll,
    RefreshAccounts,
    RefreshHistory,
    RefreshDevices,
    SaveAudioConfig {
        input_device_id: Option<String>,
        output_device_id: Option<String>,
        ringing_device_id: Option<String>,
        silent_mode: bool,
        ringtone: String,
        custom_ringtone_path: Option<String>,
        ringer_volume: u32,
    },
    SetSilentMode(bool),
    TestRingtone {
        ringtone: String,
        custom_path: String,
        volume: u32,
    },
    StopTestRingtone,
    SaveAccount {
        name: String,
        username: String,
        password: String,
        domain: String,
        registrar: String,
        port: u16,
    },
    DeleteAccount(String),
    ClearHistory,
    DeleteHistoryItem(String),
    RefreshContacts,
    SearchContacts(String),
    SyncPhonebook,
    Reconnect,
    ReconnectAccount(String),
    Quit,
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, daemon_connected)]
        #[qproperty(bool, is_registered)]
        #[qproperty(QString, account_summary)]
        #[qproperty(QString, accounts_json)]
        #[qproperty(QString, active_call_id)]
        #[qproperty(QString, active_call_state)]
        #[qproperty(QString, active_remote_party)]
        #[qproperty(i32, call_duration_seconds)]
        #[qproperty(bool, is_muted)]
        #[qproperty(QString, incoming_call_id)]
        #[qproperty(QString, incoming_caller)]
        #[qproperty(QString, call_history_json)]
        #[qproperty(QString, contacts_json)]
        #[qproperty(bool, is_syncing_contacts)]
        #[qproperty(QString, contact_sync_status)]
        #[qproperty(QString, audio_devices_json)]
        #[qproperty(QString, audio_config_json)]
        #[qproperty(QString, status_message)]
        #[qproperty(i32, raise_window_token)]
        #[qproperty(i32, toggle_window_token)]
        #[qproperty(QString, prefilled_dial_number)]
        #[qproperty(QString, reject_mode)]
        type SipUiBridge = super::SipUiBridgeRust;

        #[qinvokable]
        fn dial(self: Pin<&mut SipUiBridge>, target: QString);

        #[qinvokable]
        fn answer_call(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn reject_call(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn change_reject_mode(self: Pin<&mut SipUiBridge>, mode: QString);

        #[qinvokable]
        fn hangup_call(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn hold_call(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn resume_call(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn toggle_mute(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn send_dtmf(self: Pin<&mut SipUiBridge>, digit: QString);

        #[qinvokable]
        fn refresh_all(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn refresh_accounts(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn refresh_history(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn clear_history(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn delete_history_item(self: Pin<&mut SipUiBridge>, id: QString);

        #[qinvokable]
        fn refresh_contacts(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn search_contacts(self: Pin<&mut SipUiBridge>, query: QString);

        #[qinvokable]
        fn sync_phonebook(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn refresh_devices(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn save_audio_config(
            self: Pin<&mut SipUiBridge>,
            input_device_id: QString,
            output_device_id: QString,
            ringing_device_id: QString,
            silent_mode: bool,
            ringtone: QString,
            custom_ringtone_path: QString,
            ringer_volume: i32,
        );

        #[qinvokable]
        fn set_silent_mode(self: Pin<&mut SipUiBridge>, silent: bool);

        #[qinvokable]
        fn test_ringtone(
            self: Pin<&mut SipUiBridge>,
            ringtone: QString,
            custom_path: QString,
            volume: i32,
        );

        #[qinvokable]
        fn stop_test_ringtone(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn save_account(
            self: Pin<&mut SipUiBridge>,
            name: QString,
            username: QString,
            password: QString,
            domain: QString,
            registrar: QString,
            port: i32,
        );

        #[qinvokable]
        fn delete_account(self: Pin<&mut SipUiBridge>, account_id: QString);

        #[qinvokable]
        fn reconnect(self: Pin<&mut SipUiBridge>);

        #[qinvokable]
        fn reconnect_account(self: Pin<&mut SipUiBridge>, account_id: QString);

        #[qinvokable]
        fn quit_app(self: Pin<&mut SipUiBridge>);
    }

    impl cxx_qt::Initialize for SipUiBridge {}
    impl cxx_qt::Threading for SipUiBridge {}
}

pub struct SipUiBridgeRust {
    daemon_connected: bool,
    is_registered: bool,
    account_summary: QString,
    accounts_json: QString,
    active_call_id: QString,
    active_call_state: QString,
    active_remote_party: QString,
    call_duration_seconds: i32,
    is_muted: bool,
    incoming_call_id: QString,
    incoming_caller: QString,
    call_history_json: QString,
    contacts_json: QString,
    is_syncing_contacts: bool,
    contact_sync_status: QString,
    audio_devices_json: QString,
    audio_config_json: QString,
    status_message: QString,
    raise_window_token: i32,
    toggle_window_token: i32,
    prefilled_dial_number: QString,
    reject_mode: QString,

    cmd_tx: Option<mpsc::UnboundedSender<UiCommand>>,
}

impl Default for SipUiBridgeRust {
    fn default() -> Self {
        Self {
            daemon_connected: false,
            is_registered: false,
            account_summary: QString::from("Connecting to daemon..."),
            accounts_json: QString::from("[]"),
            active_call_id: QString::from(""),
            active_call_state: QString::from("Idle"),
            active_remote_party: QString::from(""),
            call_duration_seconds: 0,
            is_muted: false,
            incoming_call_id: QString::from(""),
            incoming_caller: QString::from(""),
            call_history_json: QString::from("[]"),
            contacts_json: QString::from("[]"),
            is_syncing_contacts: false,
            contact_sync_status: QString::from(""),
            audio_devices_json: QString::from("[]"),
            audio_config_json: QString::from("{}"),
            status_message: QString::from(""),
            raise_window_token: 0,
            toggle_window_token: 0,
            prefilled_dial_number: QString::from(""),
            reject_mode: QString::from("603_decline"),
            cmd_tx: None,
        }
    }
}

impl cxx_qt::Initialize for qobject::SipUiBridge {
    fn initialize(mut self: Pin<&mut Self>) {
        if std::env::args().any(|a| a == "--test-launch")
            || std::env::var_os("PLASMA_SIP_TEST_MODE").is_some()
        {
            // In test-launch mode, bypass D-Bus daemon connection and background thread
            // to ensure complete test isolation (no host D-Bus, network, or credential access).
            return;
        }

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UiCommand>();
        self.as_mut().rust_mut().cmd_tx = Some(cmd_tx);

        let qt_thread = self.as_ref().qt_thread();

        // Spawn dedicated background Tokio runtime for D-Bus client interactions
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    error!(error = %e, "Failed to create Tokio runtime for UI bridge");
                    return;
                }
            };

            rt.block_on(async move {
                let conn = match Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(error = %e, "Could not connect to session D-Bus");
                        let _ = qt_thread.queue(|mut qobj| {
                            qobj.as_mut().set_daemon_connected(false);
                            qobj.as_mut().set_account_summary(QString::from("Daemon offline"));
                        });
                        return;
                    }
                };

                let proxy = match PlasmaSipDaemonProxy::new(&conn).await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "Failed to create D-Bus proxy for org.kde.PlasmaSip");
                        let _ = qt_thread.queue(|mut qobj| {
                            qobj.as_mut().set_daemon_connected(false);
                            qobj.as_mut().set_account_summary(QString::from("Daemon service missing"));
                        });
                        return;
                    }
                };

                // Confirm connectivity with Ping
                if proxy.ping().await.is_ok() {
                    let _ = qt_thread.queue(|mut qobj| {
                        qobj.as_mut().set_daemon_connected(true);
                    });
                }

                // Initialize KDE Plasma StatusNotifierItem (system tray icon in panel)
                let (tray_action_tx, mut tray_action_rx) =
                    tokio::sync::mpsc::unbounded_channel::<crate::tray::TrayAction>();
                let tray_handle = crate::tray::setup_tray(&conn, tray_action_tx).await;

                // Initial data fetch
                fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;

                // Subscribe to D-Bus signals
                let mut state_changed_stream = match proxy.receive_call_state_changed().await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        warn!(error = %e, "Failed to subscribe to call_state_changed signal");
                        None
                    }
                };

                let mut incoming_stream = match proxy.receive_incoming_call().await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        warn!(error = %e, "Failed to subscribe to incoming_call signal");
                        None
                    }
                };

                let mut reg_stream = match proxy.receive_registration_changed().await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        warn!(error = %e, "Failed to subscribe to registration_changed signal");
                        None
                    }
                };

                let mut raise_stream = match proxy.receive_raise_window().await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        warn!(error = %e, "Failed to subscribe to raise_window signal");
                        None
                    }
                };

                let mut contacts_changed_stream = match proxy.receive_contacts_changed().await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        warn!(error = %e, "Failed to subscribe to contacts_changed signal");
                        None
                    }
                };

                loop {
                    tokio::select! {
                        Some(cmd) = cmd_rx.recv() => {
                            match cmd {
                                UiCommand::Dial(target) => {
                                    info!("Initiating call from GUI");
                                    match proxy.start_call("", &target).await {
                                        Ok(reply) => {
                                            if !reply.starts_with("Error") && !reply.starts_with("Invalid") {
                                                let cid = reply.clone();
                                                let tgt = target.clone();
                                                let _ = qt_thread.queue(move |mut qobj| {
                                                    qobj.as_mut().set_active_call_id(QString::from(&cid));
                                                    qobj.as_mut().set_active_remote_party(QString::from(&tgt));
                                                    qobj.as_mut().set_active_call_state(QString::from("Dialing"));
                                                    qobj.as_mut().set_call_duration_seconds(0);
                                                });
                                                if let Some(ref tray) = tray_handle {
                                                    tray.set_status(crate::tray::TrayCallStatus::ActiveCall { remote: target.clone() }).await;
                                                }
                                            } else {
                                                let err_msg = reply.clone();
                                                let _ = qt_thread.queue(move |mut qobj| {
                                                    qobj.as_mut().set_status_message(QString::from(&err_msg));
                                                });
                                            }
                                        }
                                        Err(e) => {
                                            let err_str = e.to_string();
                                            let _ = qt_thread.queue(move |mut qobj| {
                                                qobj.as_mut().set_status_message(QString::from(&err_str));
                                            });
                                        }
                                    }
                                }
                                UiCommand::Answer => {
                                    info!("Answering call from GUI");
                                    let _ = proxy.answer_call("").await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_status(crate::tray::TrayCallStatus::ActiveCall { remote: "Active".to_string() }).await;
                                    }
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::Reject => {
                                    info!("Rejecting call from GUI");
                                    let _ = proxy.reject_call("").await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_status(crate::tray::TrayCallStatus::Idle).await;
                                    }
                                    let _ = qt_thread.queue(|mut qobj| {
                                        qobj.as_mut().set_incoming_call_id(QString::from(""));
                                        qobj.as_mut().set_incoming_caller(QString::from(""));
                                    });
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::SetRejectMode(mode) => {
                                    info!(mode = %mode, "Setting call reject mode from GUI");
                                    let _ = proxy.set_reject_mode(&mode).await;
                                    let mode_clone = mode.clone();
                                    let _ = qt_thread.queue(move |mut qobj| {
                                        qobj.as_mut().set_reject_mode(QString::from(&mode_clone));
                                    });
                                }
                                UiCommand::Hangup => {
                                    info!("Hanging up call from GUI");
                                    let _ = proxy.hangup_call("").await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_status(crate::tray::TrayCallStatus::Idle).await;
                                    }
                                    let _ = qt_thread.queue(|mut qobj| {
                                        qobj.as_mut().set_active_call_state(QString::from("Idle"));
                                        qobj.as_mut().set_active_call_id(QString::from(""));
                                        qobj.as_mut().set_active_remote_party(QString::from(""));
                                        qobj.as_mut().set_call_duration_seconds(0);
                                    });
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::Hold => {
                                    info!("Holding call from GUI");
                                    let _ = proxy.hold_call("").await;
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::Resume => {
                                    info!("Resuming call from GUI");
                                    let _ = proxy.resume_call("").await;
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::ToggleMute => {
                                    let curr_muted = proxy.is_muted().await.unwrap_or(false);
                                    let new_muted = !curr_muted;
                                    let _ = proxy.set_muted(new_muted).await;
                                    let _ = qt_thread.queue(move |mut qobj| {
                                        qobj.as_mut().set_is_muted(new_muted);
                                    });
                                }
                                UiCommand::SendDtmf(digit) => {
                                    let _ = proxy.send_dtmf("", &digit.to_string()).await;
                                }
                                UiCommand::RefreshAll => {
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::RefreshAccounts => {
                                    let is_reg = update_accounts(&proxy, &qt_thread).await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_registered(is_reg).await;
                                    }
                                }
                                UiCommand::RefreshHistory => {
                                    update_history(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::RefreshContacts => {
                                    update_contacts(&proxy, &qt_thread, "").await;
                                }
                                UiCommand::SearchContacts(query) => {
                                    update_contacts(&proxy, &qt_thread, &query).await;
                                }
                                UiCommand::SyncPhonebook => {
                                    info!("Requesting phonebook sync from GUI");
                                    let _ = qt_thread.queue(|mut qobj| {
                                        qobj.as_mut().set_is_syncing_contacts(true);
                                        qobj.as_mut().set_contact_sync_status(QString::from("Synchronizing phonebook..."));
                                    });
                                    match proxy.sync_phonebook().await {
                                        Ok(res) => {
                                            let r_clone = res.clone();
                                            let _ = qt_thread.queue(move |mut qobj| {
                                                qobj.as_mut().set_is_syncing_contacts(false);
                                                qobj.as_mut().set_contact_sync_status(QString::from(&r_clone));
                                            });
                                            update_contacts(&proxy, &qt_thread, "").await;
                                        }
                                        Err(e) => {
                                            let err_msg = format!("Sync failed: {}", e);
                                            let _ = qt_thread.queue(move |mut qobj| {
                                                qobj.as_mut().set_is_syncing_contacts(false);
                                                qobj.as_mut().set_contact_sync_status(QString::from(&err_msg));
                                            });
                                        }
                                    }
                                }
                                UiCommand::RefreshDevices => {
                                    update_devices(&proxy, &qt_thread).await;
                                }
                                UiCommand::SaveAudioConfig {
                                    input_device_id,
                                    output_device_id,
                                    ringing_device_id,
                                    silent_mode,
                                    ringtone,
                                    custom_ringtone_path,
                                    ringer_volume,
                                } => {
                                    let mut cfg = sip_domain::AudioConfig::default();
                                    if let Ok(cur_json) = proxy.get_audio_config().await {
                                        if let Ok(parsed) = serde_json::from_str::<sip_domain::AudioConfig>(&cur_json) {
                                            cfg = parsed;
                                        }
                                    }
                                    cfg.input_device_id = input_device_id;
                                    cfg.output_device_id = output_device_id;
                                    cfg.ringing_device_id = ringing_device_id;
                                    cfg.silent_mode = silent_mode;
                                    cfg.ringtone = ringtone;
                                    cfg.custom_ringtone_path = custom_ringtone_path;
                                    cfg.ringer_volume = ringer_volume;
                                    if let Ok(json) = serde_json::to_string(&cfg) {
                                        info!("Sending save_audio_config to D-Bus daemon");
                                        let _ = proxy.save_audio_config(&json).await;
                                        update_devices(&proxy, &qt_thread).await;
                                    }
                                }
                                UiCommand::TestRingtone { ringtone, custom_path, volume } => {
                                    let _ = proxy.test_ringtone(&ringtone, &custom_path, volume).await;
                                }
                                UiCommand::StopTestRingtone => {
                                    let _ = proxy.stop_test_ringtone().await;
                                }
                                UiCommand::SetSilentMode(silent) => {
                                    let mut cfg = sip_domain::AudioConfig::default();
                                    if let Ok(cur_json) = proxy.get_audio_config().await {
                                        if let Ok(parsed) = serde_json::from_str::<sip_domain::AudioConfig>(&cur_json) {
                                            cfg = parsed;
                                        }
                                    }
                                    cfg.silent_mode = silent;
                                    if let Ok(json) = serde_json::to_string(&cfg) {
                                        info!(silent_mode = silent, "Updating silent mode via D-Bus daemon");
                                        let _ = proxy.save_audio_config(&json).await;
                                        update_devices(&proxy, &qt_thread).await;
                                    }
                                }
                                UiCommand::SaveAccount { name, username, password, domain, registrar, port } => {
                                    // If an account with the same username and registrar already exists, reuse its ID to update it
                                    let existing_id = match proxy.list_accounts().await {
                                        Ok(accs_json) => {
                                            serde_json::from_str::<Vec<sip_domain::SipAccount>>(&accs_json)
                                                .ok()
                                                .and_then(|list| {
                                                    list.into_iter().find(|a| a.username == username && (a.registrar == registrar || a.domain == domain)).map(|a| a.id)
                                                })
                                        }
                                        Err(_) => None,
                                    };
                                    let id = existing_id.unwrap_or_else(sip_domain::AccountId::new);
                                    let account = sip_domain::SipAccount {
                                        id,
                                        name: name.clone(),
                                        username: username.clone(),
                                        display_name: None,
                                        domain,
                                        registrar,
                                        registrar_port: port,
                                        transport: sip_domain::TransportProtocol::Udp,
                                        expires_secs: 300,
                                        enabled: true,
                                        is_default: true,
                                    };
                                    if let Ok(json) = serde_json::to_string(&account) {
                                        info!("Sending save_account to D-Bus daemon");
                                        match proxy.save_account(&json, &password).await {
                                            Ok(reply) => {
                                                info!(reply = %reply, "Daemon save_account response");
                                                if reply != "OK" {
                                                    let r_clone = reply.clone();
                                                    let _ = qt_thread.queue(move |mut qobj| {
                                                        qobj.as_mut().set_status_message(QString::from(&r_clone));
                                                    });
                                                } else {
                                                    let _ = qt_thread.queue(|mut qobj| {
                                                        qobj.as_mut().set_status_message(QString::from(""));
                                                    });
                                                    info!(account_id = %id, "Triggering immediate SIP registration for newly saved account");
                                                    let _ = proxy.register_account(&id.to_string()).await;
                                                }
                                            }
                                            Err(e) => {
                                                error!(error = %e, "Failed to call save_account over D-Bus");
                                                let err_msg = format!("D-Bus Error: {}", e);
                                                let _ = qt_thread.queue(move |mut qobj| {
                                                    qobj.as_mut().set_status_message(QString::from(&err_msg));
                                                });
                                            }
                                        }
                                        let is_reg = update_accounts(&proxy, &qt_thread).await;
                                        if let Some(ref tray) = tray_handle {
                                            tray.set_registered(is_reg).await;
                                        }
                                    }
                                }
                                UiCommand::DeleteAccount(account_id) => {
                                    let _ = proxy.delete_account(&account_id).await;
                                    let is_reg = update_accounts(&proxy, &qt_thread).await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_registered(is_reg).await;
                                    }
                                }
                                UiCommand::ClearHistory => {
                                    let _ = proxy.clear_call_history().await;
                                    update_history(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::DeleteHistoryItem(id) => {
                                    let _ = proxy.delete_call_log_entry(&id).await;
                                    update_history(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                UiCommand::Reconnect => {
                                    info!("Triggering manual SIP reconnect for all accounts from GUI");
                                    let _ = proxy.reconnect_all().await;
                                    let is_reg = update_accounts(&proxy, &qt_thread).await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_registered(is_reg).await;
                                    }
                                }
                                UiCommand::ReconnectAccount(account_id) => {
                                    info!(account_id = %account_id, "Triggering manual SIP reconnect for account from GUI");
                                    let _ = proxy.register_account(&account_id).await;
                                    let is_reg = update_accounts(&proxy, &qt_thread).await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_registered(is_reg).await;
                                    }
                                }
                                UiCommand::Quit => {
                                    break;
                                }
                            }
                        }

                        Some(action) = tray_action_rx.recv() => {
                            match action {
                                crate::tray::TrayAction::ToggleWindow => {
                                    info!("Tray action: ToggleWindow");
                                    let _ = qt_thread.queue(|mut qobj| {
                                        let curr = *qobj.as_ref().toggle_window_token();
                                        qobj.as_mut().set_toggle_window_token(curr + 1);
                                    });
                                }
                                crate::tray::TrayAction::ShowWindow => {
                                    info!("Tray action: ShowWindow");
                                    let _ = qt_thread.queue(|mut qobj| {
                                        let curr = *qobj.as_ref().raise_window_token();
                                        qobj.as_mut().set_raise_window_token(curr + 1);
                                    });
                                }
                                crate::tray::TrayAction::PrefillAndShow(target) => {
                                    info!("Tray/IPC action: Pre-fill dialer and raise window (user confirmation required)");
                                    let tgt = target.clone();
                                    let _ = qt_thread.queue(move |mut qobj| {
                                        qobj.as_mut().set_prefilled_dial_number(QString::from(&tgt));
                                        let cur = *qobj.as_ref().raise_window_token();
                                        qobj.as_mut().set_raise_window_token(cur + 1);
                                    });
                                }
                                crate::tray::TrayAction::Dial(target) => {
                                    info!("Tray action: Dial selected party");
                                    let tgt = target.clone();
                                    if let Ok(reply) = proxy.start_call("", &target).await {
                                        if !reply.starts_with("Error") && !reply.starts_with("Invalid") {
                                            let cid = reply.clone();
                                            let _ = qt_thread.queue(move |mut qobj| {
                                                qobj.as_mut().set_active_call_id(QString::from(&cid));
                                                qobj.as_mut().set_active_remote_party(QString::from(&tgt));
                                                qobj.as_mut().set_active_call_state(QString::from("Dialing"));
                                                qobj.as_mut().set_call_duration_seconds(0);
                                                let cur = *qobj.as_ref().raise_window_token();
                                                qobj.as_mut().set_raise_window_token(cur + 1);
                                            });
                                            if let Some(ref tray) = tray_handle {
                                                tray.set_status(crate::tray::TrayCallStatus::ActiveCall { remote: target }).await;
                                            }
                                        }
                                    }
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                crate::tray::TrayAction::AnswerCall => {
                                    info!("Tray action: AnswerCall");
                                    let _ = proxy.answer_call("").await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_status(crate::tray::TrayCallStatus::ActiveCall { remote: "Active".to_string() }).await;
                                    }
                                    let _ = qt_thread.queue(|mut qobj| {
                                        let curr = *qobj.as_ref().raise_window_token();
                                        qobj.as_mut().set_raise_window_token(curr + 1);
                                    });
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                crate::tray::TrayAction::HangupCall => {
                                    info!("Tray action: HangupCall");
                                    let _ = proxy.reject_call("").await;
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_status(crate::tray::TrayCallStatus::Idle).await;
                                    }
                                    fetch_and_update_state(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }
                                crate::tray::TrayAction::Quit => {
                                    std::process::exit(0);
                                }
                            }
                        }

                        Some(_sig) = async {
                            match raise_stream.as_mut() {
                                Some(s) => s.next().await,
                                None => futures::future::pending().await,
                            }
                        } => {
                            info!("Received D-Bus raise_window signal from daemon");
                            let _ = qt_thread.queue(|mut qobj| {
                                let curr = *qobj.as_ref().raise_window_token();
                                qobj.as_mut().set_raise_window_token(curr + 1);
                            });
                        }

                        Some(signal) = async {
                            match state_changed_stream.as_mut() {
                                Some(s) => s.next().await,
                                None => futures::future::pending().await,
                            }
                        } => {
                            if let Ok(args) = signal.args() {
                                let call_id = args.call_id;
                                let state_str = args.state;
                                info!(call_id = %call_id, state = %state_str, "Received D-Bus call_state_changed");

                                let normalized_str = normalize_call_state_to_str(&state_str).to_string();

                                if normalized_str == "Terminated" || normalized_str == "Idle" {
                                    if let Some(ref tray) = tray_handle {
                                        tray.set_status(crate::tray::TrayCallStatus::Idle).await;
                                    }
                                    let _ = qt_thread.queue(move |mut qobj| {
                                        qobj.as_mut().set_active_call_id(QString::from(""));
                                        qobj.as_mut().set_active_call_state(QString::from("Idle"));
                                        qobj.as_mut().set_incoming_call_id(QString::from(""));
                                        qobj.as_mut().set_incoming_caller(QString::from(""));
                                        qobj.as_mut().set_call_duration_seconds(0);
                                    });
                                } else {
                                    update_active_call(&proxy, &qt_thread, tray_handle.as_ref()).await;
                                }

                                update_history(&proxy, &qt_thread, tray_handle.as_ref()).await;
                            }
                        }

                        Some(signal) = async {
                            match incoming_stream.as_mut() {
                                Some(s) => s.next().await,
                                None => futures::future::pending().await,
                            }
                        } => {
                            if let Ok(args) = signal.args() {
                                let call_id = args.call_id;
                                let raw_caller = args.caller;
                                let parsed = sip_protocol::SipPartyInfo::parse(&raw_caller)
                                    .map(|p| p.display_string)
                                    .unwrap_or(raw_caller);
                                let caller = if let Ok(name) = proxy.resolve_caller_name(&parsed).await {
                                    if !name.is_empty() {
                                        format!("{} ({})", name, parsed)
                                    } else {
                                        parsed
                                    }
                                } else {
                                    parsed
                                };
                                info!(call_id = %call_id, "Received D-Bus incoming_call");

                                let cid_clone = call_id.clone();
                                let clr_clone = caller.clone();

                                if let Some(ref tray) = tray_handle {
                                    tray.set_status(crate::tray::TrayCallStatus::IncomingRinging { caller: clr_clone.clone() }).await;
                                }

                                let _ = qt_thread.queue(move |mut qobj| {
                                    qobj.as_mut().set_incoming_call_id(QString::from(&cid_clone));
                                    qobj.as_mut().set_incoming_caller(QString::from(&clr_clone));
                                    qobj.as_mut().set_active_remote_party(QString::from(&clr_clone));
                                    qobj.as_mut().set_active_call_state(QString::from("Ringing"));
                                    let curr = *qobj.as_ref().raise_window_token();
                                    qobj.as_mut().set_raise_window_token(curr + 1);
                                });
                            }
                        }

                        Some(signal) = async {
                            match reg_stream.as_mut() {
                                Some(s) => s.next().await,
                                None => futures::future::pending().await,
                            }
                        } => {
                            if let Ok(args) = signal.args() {
                                info!(account_id = %args.account_id, state = %args.state, "Received D-Bus registration_changed");
                                let is_reg = update_accounts(&proxy, &qt_thread).await;
                                if let Some(ref tray) = tray_handle {
                                    tray.set_registered(is_reg).await;
                                }
                            }
                        }

                        Some(_sig) = async {
                            match contacts_changed_stream.as_mut() {
                                Some(s) => s.next().await,
                                None => futures::future::pending().await,
                            }
                        } => {
                            info!("Received D-Bus contacts_changed signal from daemon");
                            update_contacts(&proxy, &qt_thread, "").await;
                        }
                    }
                }
            });
        });
    }
}

async fn fetch_and_update_state(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
    tray_handle: Option<&crate::tray::TrayHandle>,
) {
    let is_reg = update_accounts(proxy, qt_thread).await;
    if let Some(tray) = tray_handle {
        tray.set_registered(is_reg).await;
    }
    update_active_call(proxy, qt_thread, tray_handle).await;
    update_history(proxy, qt_thread, tray_handle).await;
    update_contacts(proxy, qt_thread, "").await;
    update_devices(proxy, qt_thread).await;
    update_reject_mode(proxy, qt_thread).await;
}

async fn update_accounts(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
) -> bool {
    if let Ok(accounts_json) = proxy.list_accounts().await {
        let mut is_registered = false;
        let mut summary = "No accounts configured".to_string();

        if let Ok(accounts) = serde_json::from_str::<Vec<sip_domain::SipAccount>>(&accounts_json) {
            if let Some(acc) = accounts.iter().find(|a| a.enabled) {
                let reg_status = match proxy.get_account_status(&acc.id.to_string()).await {
                    Ok(st) => st,
                    Err(_) => "\"unregistered\"".to_string(),
                };

                let state_opt =
                    serde_json::from_str::<sip_domain::RegistrationState>(&reg_status).ok();
                match state_opt {
                    Some(sip_domain::RegistrationState::Registered { .. }) => {
                        summary =
                            format!("Registered: {} ({}@{})", acc.name, acc.username, acc.domain);
                        is_registered = true;
                    }
                    Some(sip_domain::RegistrationState::Registering) => {
                        summary = format!("Registering: {}@{}...", acc.username, acc.domain);
                        is_registered = false;
                    }
                    Some(sip_domain::RegistrationState::Failed { reason, .. }) => {
                        summary = format!("Registration Failed: {}", reason);
                        is_registered = false;
                    }
                    Some(sip_domain::RegistrationState::Unregistering) => {
                        summary = format!("Unregistering: {}@{}...", acc.username, acc.domain);
                        is_registered = false;
                    }
                    Some(sip_domain::RegistrationState::Unregistered) | None => {
                        let lower = reg_status.to_lowercase();
                        if lower.contains("registered") && !lower.contains("unregistered") {
                            summary = format!(
                                "Registered: {} ({}@{})",
                                acc.name, acc.username, acc.domain
                            );
                            is_registered = true;
                        } else if lower.contains("registering") {
                            summary = format!("Registering: {}@{}...", acc.username, acc.domain);
                            is_registered = false;
                        } else if lower.contains("failed") {
                            summary = format!(
                                "Registration Failed: {} (check credentials)",
                                acc.username
                            );
                            is_registered = false;
                        } else {
                            summary = format!(
                                "Configured: {} ({}@{})",
                                acc.name, acc.username, acc.domain
                            );
                            is_registered = false;
                        }
                    }
                }
            }
        }

        let summary_str = summary;
        let json_str = accounts_json;
        let _ = qt_thread.queue(move |mut qobj| {
            qobj.as_mut().set_is_registered(is_registered);
            qobj.as_mut()
                .set_account_summary(QString::from(&summary_str));
            qobj.as_mut().set_accounts_json(QString::from(&json_str));
        });
        is_registered
    } else {
        false
    }
}

fn normalize_call_state_to_str(raw: &str) -> &'static str {
    if let Ok(st) = serde_json::from_str::<sip_domain::CallState>(raw) {
        match st {
            sip_domain::CallState::Idle => "Idle",
            sip_domain::CallState::Dialing => "Dialing",
            sip_domain::CallState::RingingRemote | sip_domain::CallState::RingingLocal => "Ringing",
            sip_domain::CallState::Active { .. } => "Active",
            sip_domain::CallState::Held => "Held",
            sip_domain::CallState::Terminating { .. }
            | sip_domain::CallState::Terminated { .. } => "Terminated",
        }
    } else {
        let s = raw.to_lowercase();
        if s.contains("active") {
            "Active"
        } else if s.contains("held") {
            "Held"
        } else if s.contains("ringing") || s.contains("early") {
            "Ringing"
        } else if s.contains("dialing") {
            "Dialing"
        } else if s.contains("terminat") {
            "Terminated"
        } else {
            "Idle"
        }
    }
}

async fn update_active_call(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
    tray_handle: Option<&crate::tray::TrayHandle>,
) {
    if let Ok(call_json) = proxy.get_active_call().await {
        if call_json != "null" && !call_json.is_empty() {
            if let Ok(active) = serde_json::from_str::<sip_domain::ActiveCallInfo>(&call_json) {
                let cid = active.id.to_string();
                let remote = active.remote_party.clone();
                let normalized_state = match active.state {
                    sip_domain::CallState::Idle => "Idle",
                    sip_domain::CallState::Dialing => "Dialing",
                    sip_domain::CallState::RingingRemote | sip_domain::CallState::RingingLocal => {
                        "Ringing"
                    }
                    sip_domain::CallState::Active { .. } => "Active",
                    sip_domain::CallState::Held => "Held",
                    sip_domain::CallState::Terminating { .. }
                    | sip_domain::CallState::Terminated { .. } => "Terminated",
                };
                let duration_secs = active
                    .connected_at
                    .map(|c| (chrono::Utc::now() - c).num_seconds().max(0) as i32)
                    .unwrap_or(0);
                let is_muted = active.muted;

                if let Some(tray) = tray_handle {
                    match normalized_state {
                        "Active" => {
                            tray.set_status(crate::tray::TrayCallStatus::ActiveCall {
                                remote: remote.clone(),
                            })
                            .await;
                        }
                        "Held" => {
                            tray.set_status(crate::tray::TrayCallStatus::ActiveCall {
                                remote: format!("{} (Held)", remote),
                            })
                            .await;
                        }
                        "Ringing" => {
                            tray.set_status(crate::tray::TrayCallStatus::IncomingRinging {
                                caller: remote.clone(),
                            })
                            .await;
                        }
                        "Terminated" | "Idle" => {
                            tray.set_status(crate::tray::TrayCallStatus::Idle).await;
                        }
                        _ => {}
                    }
                }

                let _ = qt_thread.queue(move |mut qobj| {
                    qobj.as_mut().set_active_call_id(QString::from(&cid));
                    qobj.as_mut()
                        .set_active_remote_party(QString::from(&remote));
                    qobj.as_mut()
                        .set_active_call_state(QString::from(normalized_state));
                    qobj.as_mut().set_call_duration_seconds(duration_secs);
                    qobj.as_mut().set_is_muted(is_muted);
                });
                return;
            }
        }
    }

    if let Some(tray) = tray_handle {
        tray.set_status(crate::tray::TrayCallStatus::Idle).await;
    }
}

async fn update_history(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
    tray_handle: Option<&crate::tray::TrayHandle>,
) {
    if let Ok(history_json) = proxy.get_call_history(50).await {
        if let Some(tray) = tray_handle {
            tray.update_call_history(&history_json).await;
        }
        let json_str = history_json;
        let _ = qt_thread.queue(move |mut qobj| {
            qobj.as_mut()
                .set_call_history_json(QString::from(&json_str));
        });
    }
}

async fn update_contacts(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
    query: &str,
) {
    let res = if query.trim().is_empty() {
        proxy.list_contacts().await
    } else {
        proxy.search_contacts(query.trim()).await
    };
    if let Ok(contacts_json) = res {
        let json_str = contacts_json;
        let _ = qt_thread.queue(move |mut qobj| {
            qobj.as_mut().set_contacts_json(QString::from(&json_str));
        });
    }
}

async fn update_devices(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
) {
    if let Ok(devices_json) = proxy.get_audio_devices().await {
        let json_str = devices_json;
        let _ = qt_thread.queue(move |mut qobj| {
            qobj.as_mut()
                .set_audio_devices_json(QString::from(&json_str));
        });
    }
    if let Ok(cfg_json) = proxy.get_audio_config().await {
        let json_str = cfg_json;
        let _ = qt_thread.queue(move |mut qobj| {
            qobj.as_mut()
                .set_audio_config_json(QString::from(&json_str));
        });
    }
}

async fn update_reject_mode(
    proxy: &PlasmaSipDaemonProxy<'_>,
    qt_thread: &cxx_qt::CxxQtThread<qobject::SipUiBridge>,
) {
    if let Ok(mode) = proxy.get_reject_mode().await {
        let mode_q = QString::from(&mode);
        let _ = qt_thread.queue(move |mut qobj| {
            qobj.as_mut().set_reject_mode(mode_q);
        });
    }
}

impl qobject::SipUiBridge {
    pub fn dial(self: Pin<&mut Self>, target: QString) {
        let s = target.to_string();
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Dial(s));
        }
    }

    pub fn answer_call(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Answer);
        }
    }

    pub fn reject_call(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Reject);
        }
    }

    pub fn change_reject_mode(self: Pin<&mut Self>, mode: QString) {
        let m = mode.to_string();
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::SetRejectMode(m));
        }
    }

    pub fn hangup_call(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Hangup);
        }
    }

    pub fn hold_call(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Hold);
        }
    }

    pub fn resume_call(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Resume);
        }
    }

    pub fn toggle_mute(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::ToggleMute);
        }
    }

    pub fn send_dtmf(self: Pin<&mut Self>, digit: QString) {
        if let Some(c) = digit.to_string().chars().next() {
            if let Some(ref tx) = self.rust().cmd_tx {
                let _ = tx.send(UiCommand::SendDtmf(c));
            }
        }
    }

    pub fn refresh_all(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::RefreshAll);
        }
    }

    pub fn refresh_accounts(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::RefreshAccounts);
        }
    }

    pub fn refresh_history(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::RefreshHistory);
        }
    }

    pub fn clear_history(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::ClearHistory);
        }
    }

    pub fn delete_history_item(self: Pin<&mut Self>, id: QString) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::DeleteHistoryItem(id.to_string()));
        }
    }

    pub fn refresh_contacts(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::RefreshContacts);
        }
    }

    pub fn search_contacts(self: Pin<&mut Self>, query: QString) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::SearchContacts(query.to_string()));
        }
    }

    pub fn sync_phonebook(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::SyncPhonebook);
        }
    }

    pub fn refresh_devices(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::RefreshDevices);
        }
    }

    pub fn save_audio_config(
        self: Pin<&mut Self>,
        input_device_id: QString,
        output_device_id: QString,
        ringing_device_id: QString,
        silent_mode: bool,
        ringtone: QString,
        custom_ringtone_path: QString,
        ringer_volume: i32,
    ) {
        let in_str = input_device_id.to_string();
        let out_str = output_device_id.to_string();
        let ring_str = ringing_device_id.to_string();
        let ringtone_str = ringtone.to_string();
        let custom_str = custom_ringtone_path.to_string();

        let in_opt = if in_str.is_empty() || in_str == "default" {
            None
        } else {
            Some(in_str)
        };
        let out_opt = if out_str.is_empty() || out_str == "default" {
            None
        } else {
            Some(out_str)
        };
        let ring_opt = if ring_str.is_empty() || ring_str == "default" {
            None
        } else {
            Some(ring_str)
        };
        let custom_opt = if custom_str.trim().is_empty() {
            None
        } else {
            Some(custom_str.trim().to_string())
        };

        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::SaveAudioConfig {
                input_device_id: in_opt,
                output_device_id: out_opt,
                ringing_device_id: ring_opt,
                silent_mode,
                ringtone: if ringtone_str.is_empty() {
                    "marimba".into()
                } else {
                    ringtone_str
                },
                custom_ringtone_path: custom_opt,
                ringer_volume: ringer_volume.clamp(0, 100) as u32,
            });
        }
    }

    pub fn test_ringtone(
        self: Pin<&mut Self>,
        ringtone: QString,
        custom_path: QString,
        volume: i32,
    ) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::TestRingtone {
                ringtone: ringtone.to_string(),
                custom_path: custom_path.to_string(),
                volume: volume.clamp(0, 100) as u32,
            });
        }
    }

    pub fn stop_test_ringtone(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::StopTestRingtone);
        }
    }

    pub fn set_silent_mode(self: Pin<&mut Self>, silent: bool) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::SetSilentMode(silent));
        }
    }

    pub fn save_account(
        self: Pin<&mut Self>,
        name: QString,
        username: QString,
        password: QString,
        domain: QString,
        registrar: QString,
        port: i32,
    ) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::SaveAccount {
                name: name.to_string(),
                username: username.to_string(),
                password: password.to_string(),
                domain: domain.to_string(),
                registrar: registrar.to_string(),
                port: port as u16,
            });
        }
    }

    pub fn delete_account(self: Pin<&mut Self>, account_id: QString) {
        let s = account_id.to_string();
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::DeleteAccount(s));
        }
    }

    pub fn reconnect(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Reconnect);
        }
    }

    pub fn reconnect_account(self: Pin<&mut Self>, account_id: QString) {
        let s = account_id.to_string();
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::ReconnectAccount(s));
        }
    }

    pub fn quit_app(self: Pin<&mut Self>) {
        if let Some(ref tx) = self.rust().cmd_tx {
            let _ = tx.send(UiCommand::Quit);
        }
    }
}
