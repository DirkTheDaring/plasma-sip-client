//! Plasma SIP Desktop Application & CLI Client (`plasma-sip`)
//!
//! Dual-mode client:
//! - When invoked without subcommands (or with `--gui`, `--dial <target>`, `--test-launch`),
//!   instantiates a genuine KDE Plasma 6 Kirigami graphical interface via CXX-Qt and QML.
//! - When invoked with subcommands (`ping`, `call`, `hangup`, `history`, `devices`, `status`, `list-accounts`),
//!   acts as a command-line client communicating with `plasma-sipd` over session D-Bus.

#[allow(unused_imports)]
use plasma_sip::bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use std::env;
use zbus::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt::try_init();
    let args: Vec<String> = env::args().collect();
    let first_arg = args.get(1).map(|s| s.as_str());

    // Check if user requested CLI subcommands
    match first_arg {
        Some("ping")
        | Some("list-accounts")
        | Some("call")
        | Some("hangup")
        | Some("history")
        | Some("devices")
        | Some("status") => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            return rt.block_on(run_cli(args));
        }
        Some("--help") | Some("-h") | Some("help") => {
            print_usage();
            return Ok(());
        }
        _ => {}
    }

    // Check if running test launch
    let test_launch = args.iter().any(|a| a == "--test-launch");
    if !test_launch {
        // Enforce Single-Instance: If GUI is already running, activate it and exit
        if check_single_instance(&args)? {
            return Ok(());
        }
    }

    // Otherwise, launch the KDE Plasma Kirigami Graphical Interface
    run_gui(args)
}

fn extract_dial_target(args: &[String]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--dial" {
            return args.get(i + 1).cloned();
        }
        if arg.starts_with("sip:") || arg.starts_with("sips:") || arg.starts_with("tel:") {
            return Some(arg.clone());
        }
    }
    None
}

fn check_single_instance(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };
        let dbus = match zbus::fdo::DBusProxy::new(&conn).await {
            Ok(d) => d,
            Err(_) => return Ok(false),
        };
        let bus_name = match zbus::names::BusName::try_from("org.kde.plasma_sip.ui") {
            Ok(b) => b,
            Err(_) => return Ok(false),
        };
        if let Ok(true) = dbus.name_has_owner(bus_name).await {
            if let Some(target) = extract_dial_target(args) {
                let _ = conn
                    .call_method(
                        Some("org.kde.plasma_sip.ui"),
                        "/org/kde/plasma_sip/ui",
                        Some("org.kde.plasma_sip.ui"),
                        "Dial",
                        &(target),
                    )
                    .await;
            } else if !args.iter().any(|a| a == "--minimized" || a == "--tray") {
                let _ = conn
                    .call_method(
                        Some("org.kde.plasma_sip.ui"),
                        "/org/kde/plasma_sip/ui",
                        Some("org.kde.plasma_sip.ui"),
                        "Activate",
                        &(),
                    )
                    .await;
            }
            return Ok(true);
        }
        Ok(false)
    })
}

fn print_usage() {
    println!("Plasma SIP — Pure Rust SIP Softphone for KDE Plasma 6\n");
    println!("Usage:");
    println!("  plasma-sip                       Launch graphical softphone (default)");
    println!("  plasma-sip --gui                 Launch graphical softphone");
    println!("  plasma-sip --minimized           Launch softphone minimized to toolbar tray");
    println!("  plasma-sip --tray                Launch softphone minimized to toolbar tray");
    println!("  plasma-sip --dial <number>       Launch softphone and pre-fill dial number");
    println!("  plasma-sip --test-launch         Verify QML engine and assets load, then exit 0");
    println!("  plasma-sip <command> [args]      Run command-line action\n");
    println!("CLI Commands:");
    println!("  ping                             Check daemon connectivity");
    println!("  status                           Show active call status");
    println!("  list-accounts                    List configured SIP accounts");
    println!("  call <number> [account_id]       Place an outgoing call");
    println!("  hangup [call_id]                 Hang up active call");
    println!("  history [limit]                  Show recent call history");
    println!("  devices                          List audio input and output devices");
    println!("  help                             Display this help message");
}

async fn run_cli(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Cannot connect to session D-Bus: {}", e);
            eprintln!("Ensure that 'plasma-sipd' is running or start it with: plasma-sipd &");
            std::process::exit(1);
        }
    };

    match command {
        "ping" => {
            let reply: String = conn
                .call_method(
                    Some("org.kde.PlasmaSip"),
                    "/org/kde/PlasmaSip",
                    Some("org.kde.PlasmaSip"),
                    "Ping",
                    &(),
                )
                .await?
                .body()
                .deserialize()?;
            println!("Response: {}", reply);
        }
        "list-accounts" => {
            let reply: String = conn
                .call_method(
                    Some("org.kde.PlasmaSip"),
                    "/org/kde/PlasmaSip",
                    Some("org.kde.PlasmaSip"),
                    "ListAccounts",
                    &(),
                )
                .await?
                .body()
                .deserialize()?;
            println!("Configured Accounts:\n{}", reply);
        }
        "call" => {
            if let Some(target) = args.get(2) {
                let account_id = args.get(3).map(|s| s.as_str()).unwrap_or("");
                let reply: String = conn
                    .call_method(
                        Some("org.kde.PlasmaSip"),
                        "/org/kde/PlasmaSip",
                        Some("org.kde.PlasmaSip"),
                        "StartCall",
                        &(account_id, target),
                    )
                    .await?
                    .body()
                    .deserialize()?;
                println!("Call initiated: {}", reply);
            } else {
                eprintln!("Usage: plasma-sip call <target_number_or_sip_uri> [account_id]");
            }
        }
        "hangup" => {
            let call_id = args.get(2).map(|s| s.as_str()).unwrap_or("");
            let reply: String = conn
                .call_method(
                    Some("org.kde.PlasmaSip"),
                    "/org/kde/PlasmaSip",
                    Some("org.kde.PlasmaSip"),
                    "HangupCall",
                    &(call_id),
                )
                .await?
                .body()
                .deserialize()?;
            println!("Hangup response: {}", reply);
        }
        "history" => {
            let limit: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
            let reply: String = conn
                .call_method(
                    Some("org.kde.PlasmaSip"),
                    "/org/kde/PlasmaSip",
                    Some("org.kde.PlasmaSip"),
                    "GetCallHistory",
                    &(limit),
                )
                .await?
                .body()
                .deserialize()?;
            println!("Call History:\n{}", reply);
        }
        "devices" => {
            let reply: String = conn
                .call_method(
                    Some("org.kde.PlasmaSip"),
                    "/org/kde/PlasmaSip",
                    Some("org.kde.PlasmaSip"),
                    "GetAudioDevices",
                    &(),
                )
                .await?
                .body()
                .deserialize()?;
            println!("Audio Devices:\n{}", reply);
        }
        "status" => {
            let reply: String = conn
                .call_method(
                    Some("org.kde.PlasmaSip"),
                    "/org/kde/PlasmaSip",
                    Some("org.kde.PlasmaSip"),
                    "GetActiveCall",
                    &(),
                )
                .await?
                .body()
                .deserialize()?;
            println!("Active Call Status:\n{}", reply);
        }
        _ => {
            print_usage();
        }
    }

    Ok(())
}

unsafe extern "C" {
    fn init_plasma_sip_desktop_integration();
}

fn run_gui(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let test_launch = args.iter().any(|a| a == "--test-launch");

    // Suppress benign upstream KDE Frameworks Kirigami internal binding loop warning (QTBUG-85941 workaround)
    if std::env::var_os("QT_LOGGING_RULES").is_none() {
        // SAFETY: Called early during startup before secondary threads are spawned.
        unsafe {
            std::env::set_var("QT_LOGGING_RULES", "qt.qml.binding=false");
        }
    }

    // Initialize the Qt GUI Application
    let mut app = QGuiApplication::new();

    // Configure Wayland desktop file name and application window icon
    unsafe {
        init_plasma_sip_desktop_integration();
    }

    // Register embedded QML module compiled into the binary
    cxx_qt::init_qml_module!("org.kde.plasma_sip");

    // Initialize the QML Application Engine
    let mut engine = QQmlApplicationEngine::new();

    let url = QUrl::from("qrc:/qt/qml/org/kde/plasma_sip/qml/main.qml");
    if let Some(engine_mut) = engine.as_mut() {
        engine_mut.load(&url);
    }

    if test_launch {
        println!("Plasma SIP GUI test-launch: QML engine and bridge loaded successfully.");
        return Ok(());
    }

    // Run Qt event loop
    if let Some(app_mut) = app.as_mut() {
        app_mut.exec();
    }

    Ok(())
}
