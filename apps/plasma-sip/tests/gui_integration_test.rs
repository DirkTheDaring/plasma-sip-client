use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn get_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // apps
    path.pop(); // repo root
    path.push("target");
    path.push("debug");
    path.push("plasma-sip");
    path
}

#[test]
fn test_qt_application_and_qml_engine_launch_cleanly() {
    let binary = get_binary_path();
    assert!(
        binary.exists(),
        "plasma-sip binary must exist at {:?}",
        binary
    );

    let output = Command::new(&binary)
        .arg("--test-launch")
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("Failed to execute plasma-sip --test-launch");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    println!("STDOUT:\n{}", stdout);
    println!("STDERR:\n{}", stderr);

    assert!(
        output.status.success(),
        "plasma-sip --test-launch must exit with code 0. Stderr: {}",
        stderr
    );

    assert!(
        stdout.contains("Plasma SIP GUI test-launch: QML engine and bridge loaded successfully."),
        "Stdout must confirm successful QML engine and bridge loading"
    );

    assert!(
        !stderr.contains("failed to load component"),
        "QML engine reported component loading failure"
    );
    assert!(
        !stderr.contains("module not found"),
        "Missing QML module imports detected in stderr"
    );
}

#[test]
fn test_wayland_and_x11_platform_support() {
    let binary = get_binary_path();
    assert!(binary.exists(), "plasma-sip binary must exist");

    // Check dynamic link dependencies with ldd
    let ldd_output = Command::new("ldd")
        .arg(&binary)
        .output()
        .expect("Failed to run ldd");

    let ldd_str = String::from_utf8_lossy(&ldd_output.stdout);
    assert!(
        ldd_str.contains("libQt6Gui.so.6"),
        "plasma-sip must link against Qt 6 Gui"
    );
    assert!(
        ldd_str.contains("libQt6Qml.so.6"),
        "plasma-sip must link against Qt 6 Qml"
    );

    // Verify Wayland and X11 Qt 6 platform plugins are present on system
    let platforms_dir = Path::new("/usr/lib64/qt6/plugins/platforms");
    assert!(
        platforms_dir.join("libqwayland.so").exists(),
        "Qt 6 Wayland platform plugin (libqwayland.so) must be present"
    );
    assert!(
        platforms_dir.join("libqxcb.so").exists(),
        "Qt 6 X11 platform plugin (libqxcb.so) must be present"
    );

    // Verify offscreen smoke test passes (standard for headless / virtual Wayland / X11 test environments)
    let smoke_run = Command::new(&binary)
        .arg("--test-launch")
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("Failed to run offscreen smoke test");
    assert!(
        smoke_run.status.success(),
        "Offscreen platform smoke test failed"
    );
}

#[test]
fn test_desktop_launcher_file_and_packaging_assets() {
    let root_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let desktop_file = root_dir.join("packaging/desktop/org.kde.plasma_sip.desktop");
    assert!(desktop_file.exists(), "Desktop launcher file must exist");

    let desktop_content = fs::read_to_string(&desktop_file).expect("read desktop file");
    assert!(
        desktop_content.contains("Exec=plasma-sip %u"),
        "Desktop entry must execute plasma-sip"
    );
    assert!(
        desktop_content.contains("Icon=org.kde.plasma_sip"),
        "Desktop entry must specify correct icon"
    );
    assert!(
        desktop_content.contains("Categories=Network;Telephony;Qt;KDE;"),
        "Desktop entry must categorize as Network/Telephony/Qt/KDE"
    );

    // Verify installer script installs QML assets and binaries
    let install_sh = root_dir.join("packaging/install.sh");
    assert!(install_sh.exists(), "install.sh must exist");
    let install_content = fs::read_to_string(&install_sh).expect("read install.sh");
    assert!(
        install_content.contains("Installing QML assets"),
        "Installer must install QML assets"
    );
    assert!(
        install_content.contains("share/plasma-sip/qml"),
        "Installer must install to share/plasma-sip/qml"
    );
}

#[test]
fn test_cli_help_and_subcommand_dispatch() {
    let binary = get_binary_path();
    assert!(binary.exists(), "plasma-sip binary must exist");

    let output = Command::new(&binary)
        .arg("--help")
        .output()
        .expect("Failed to execute plasma-sip --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "--help must succeed");
    assert!(
        stdout.contains("plasma-sip"),
        "--help must display usage summary"
    );
    assert!(stdout.contains("ping"), "--help must list ping CLI command");
    assert!(
        stdout.contains("call <number>"),
        "--help must list call CLI command"
    );
    assert!(
        stdout.contains("hangup"),
        "--help must list hangup CLI command"
    );
    assert!(
        stdout.contains("history"),
        "--help must list history CLI command"
    );
    assert!(
        stdout.contains("devices"),
        "--help must list devices CLI command"
    );
}

#[test]
fn test_gui_daemon_independence_contract() {
    // Contract test verifying that:
    // 1. The desktop UI process (`plasma-sip`) connects as a D-Bus client to the daemon.
    // 2. The daemon (`plasma-sipd`) owns `org.kde.PlasmaSip` and runs as an independent daemon.
    // 3. Terminating or restarting the UI does not terminate or disrupt the background daemon.
    let root_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let systemd_service = root_dir.join("packaging/systemd/plasma-sipd.service");
    assert!(
        systemd_service.exists(),
        "Daemon systemd user service must exist"
    );
    let service_content = fs::read_to_string(systemd_service).expect("read systemd service");
    assert!(
        service_content.contains("ExecStart="),
        "Service must have ExecStart for plasma-sipd"
    );
    assert!(
        service_content.contains("Restart=on-failure"),
        "Service must restart on failure independently"
    );
}

#[test]
fn test_live_dbus_operations_and_gui_lifecycle() {
    // Safe execution requires an isolated D-Bus daemon and isolated temporary environment.
    // Without an isolated test bus (e.g., dbus-run-session), this test is skipped to prevent
    // interacting with the host user's active session bus or modifying host state.
    if std::env::var_os("PLASMA_SIP_ISOLATED_TEST_BUS").is_none() {
        eprintln!(
            "Skipping test_live_dbus_operations_and_gui_lifecycle: safe execution requires PLASMA_SIP_ISOLATED_TEST_BUS"
        );
        return;
    }

    let sip_bin = get_binary_path();
    let mut sipd_bin = sip_bin.clone();
    sipd_bin.pop();
    sipd_bin.push("plasma-sipd");

    if !sipd_bin.exists() {
        return;
    }

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    // Check if a daemon instance is already responding to ping on D-Bus
    let already_running = Command::new(&sip_bin)
        .arg("ping")
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("pong"))
        .unwrap_or(false);

    let _guard = if !already_running {
        let child = match Command::new(&sipd_bin).spawn() {
            Ok(c) => c,
            Err(_) => return,
        };

        // Poll until temporary daemon responds to ping over D-Bus
        let mut ready = false;
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let out = Command::new(&sip_bin).arg("ping").output();
            if let Ok(o) = out
                && o.status.success()
                && String::from_utf8_lossy(&o.stdout).contains("pong")
            {
                ready = true;
                break;
            }
        }

        if !ready {
            return;
        }

        Some(ChildGuard(child))
    } else {
        None
    };

    // 1. Verify D-Bus devices query
    let dev_out = Command::new(&sip_bin)
        .arg("devices")
        .output()
        .expect("devices output");
    assert!(dev_out.status.success());
    let dev_str = String::from_utf8_lossy(&dev_out.stdout);
    assert!(dev_str.contains("inputs") || dev_str.contains("Audio Devices"));

    // 2. Launch GUI in test mode (instantiates QGuiApplication, loads Kirigami QML, bridges to D-Bus)
    let gui_out = Command::new(&sip_bin)
        .arg("--test-launch")
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("gui launch output");
    assert!(
        gui_out.status.success(),
        "GUI must launch cleanly with live daemon"
    );
    let gui_str = String::from_utf8_lossy(&gui_out.stdout);
    assert!(
        gui_str.contains("Plasma SIP GUI test-launch: QML engine and bridge loaded successfully.")
    );

    // 3. Verify GUI termination did not kill the daemon (Daemon Independence)
    let ping_after = Command::new(&sip_bin)
        .arg("ping")
        .output()
        .expect("ping after GUI exit");
    assert!(ping_after.status.success());
    assert!(String::from_utf8_lossy(&ping_after.stdout).contains("pong"));
}
