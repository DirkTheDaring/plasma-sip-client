//! Plasma SIP Daemon (plasma-sipd)
//! Headless user-session background service providing telephony and D-Bus IPC.

use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use zbus::connection::Builder;

use sip_service::{AccountManager, CallManager, PlasmaSipDbusService};
use sip_storage::StorageManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Mandatory Security Invariant: Never run as root
    assert_not_running_as_root();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Plasma SIP background service (plasma-sipd)");

    // 2. Determine data directory: ~/.local/share/plasma-sip/
    let data_dir = dirs_fallback_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700));
    }
    let db_path = data_dir.join("plasma-sip.db");

    info!("Opening SQLite storage");
    let storage = Arc::new(StorageManager::new_with_path(&db_path)?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if db_path.exists() {
            let _ = std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600));
        }
    }

    // 3. Bind SIP UDP Transport and start receive loop
    let bind_addr = std::env::var("PLASMA_SIP_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:5060".into());
    let transport = match sip_transport::SipUdpTransport::bind(&bind_addr).await {
        Ok(t) => Arc::new(t),
        Err(e) => {
            warn!(
                error = %e,
                bind_addr = %bind_addr,
                "Failed to bind configured SIP port, falling back to ephemeral port 0.0.0.0:0"
            );
            Arc::new(sip_transport::SipUdpTransport::bind("0.0.0.0:0").await?)
        }
    };
    let transactions = Arc::new(sip_protocol::TransactionManager::new());
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

    // 4. Connect to Session D-Bus bus with duplicate instance prevention
    info!("Connecting to Session D-Bus bus as org.kde.PlasmaSip");
    let conn_res = Builder::session()?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .name("org.kde.PlasmaSip")?
        .serve_at("/org/kde/PlasmaSip", dbus_service)?
        .build()
        .await;

    let conn = match conn_res {
        Ok(c) => c,
        Err(e) => {
            error!(
                error = %e,
                "Failed to register D-Bus name org.kde.PlasmaSip. Another plasma-sipd instance may already be running."
            );
            return Err(e.into());
        }
    };

    // Monitor for unexpected D-Bus NameLost signal
    let dbus_proxy = zbus::fdo::DBusProxy::new(&conn).await.ok();
    let name_lost_stream = match &dbus_proxy {
        Some(p) => p.receive_name_lost().await.ok(),
        None => None,
    };

    // Forward registration state transitions to D-Bus clients as signals
    let _reg_signal_task = PlasmaSipDbusService::spawn_registration_signal_forwarder(
        conn.clone(),
        "/org/kde/PlasmaSip".to_string(),
        account_manager.clone(),
    );

    // Forward call state transitions to D-Bus clients as signals
    let _call_signal_task = PlasmaSipDbusService::spawn_call_signal_forwarder(
        conn.clone(),
        "/org/kde/PlasmaSip".to_string(),
        call_manager.clone(),
    );

    // Initialize desktop notifications and notification action listener
    let notifier = sip_platform::PlatformNotifier::with_connection(Some(conn.clone()));
    call_manager.set_notifier(notifier.clone()).await;

    let emitter_opt = zbus::object_server::SignalEmitter::new(&conn, "/org/kde/PlasmaSip").ok();
    let mut action_rx = notifier.subscribe_actions();
    let call_manager_action_runner = call_manager.clone();
    let _action_task = tokio::spawn(async move {
        while let Ok((call_id, action)) = action_rx.recv().await {
            match action {
                sip_platform::NotificationAction::Open => {
                    info!(call_id = %call_id, "User requested to open call from desktop notification");
                    if let Some(ref em) = emitter_opt {
                        let _ = PlasmaSipDbusService::raise_window(em).await;
                    }
                }
                sip_platform::NotificationAction::Answer => {
                    info!(call_id = %call_id, "User answered call via desktop notification");
                    let _ = call_manager_action_runner.answer_call(call_id).await;
                    if let Some(ref em) = emitter_opt {
                        let _ = PlasmaSipDbusService::raise_window(em).await;
                    }
                }
                sip_platform::NotificationAction::Reject => {
                    info!(call_id = %call_id, "User declined call via desktop notification");
                    let _ = call_manager_action_runner.reject_call(call_id).await;
                }
            }
        }
    });

    info!("Plasma SIP Daemon is now ready and listening on D-Bus: org.kde.PlasmaSip");

    // 5. Service restart recovery: restore and register enabled accounts via SIP network
    match account_manager.recover_and_register_all().await {
        Ok(count) => {
            info!(
                recovered_accounts = count,
                "Account recovery completed on daemon startup"
            );
        }
        Err(e) => {
            warn!(error = %e, "Failed to recover accounts during startup");
        }
    }

    // 6. Controlled shutdown: listen for SIGINT (Ctrl-C), SIGTERM, or D-Bus NameLost
    #[cfg(unix)]
    {
        use futures::StreamExt;
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT. Initiating graceful shutdown.");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM. Initiating graceful shutdown.");
            }
            _ = async {
                if let Some(mut stream) = name_lost_stream {
                    while let Some(sig) = stream.next().await {
                        if let Ok(args) = sig.args() {
                            if args.name == "org.kde.PlasmaSip" {
                                error!("Lost ownership of D-Bus name org.kde.PlasmaSip. Exiting.");
                                return;
                            }
                        }
                    }
                } else {
                    futures::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        info!("Received shutdown signal. Initiating graceful shutdown.");
    }

    // Gracefully unregister all enabled accounts on shutdown
    if let Ok(accounts) = account_manager.list_accounts() {
        for acc in accounts {
            if acc.enabled {
                let _ = account_manager.unregister_account(acc.id).await;
            }
        }
    }

    info!("Clean shutdown complete. Goodbye!");
    Ok(())
}

fn assert_not_running_as_root() {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("Uid:") {
                let uids: Vec<&str> = line.split_whitespace().collect();
                if uids.len() > 1 && uids[1] == "0" {
                    eprintln!("CRITICAL ERROR: plasma-sipd must never run as root. Exiting.");
                    std::process::exit(1);
                }
            }
        }
    }
}

fn dirs_fallback_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("PLASMA_SIP_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("plasma-sip")
    } else {
        std::path::PathBuf::from("./data")
    }
}
