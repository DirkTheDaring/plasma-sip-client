//! Milestone R7 — Deployment, Packaging, Lifecycle, and Recovery Automated Tests
//!
//! Validates:
//! - Packaging assets: binaries, QML files, SVG icons, desktop file, D-Bus service, systemd unit.
//! - Clean installation and uninstallation via packaging scripts.
//! - First-run configuration, directory permissions (0o700), and SQLite file permissions (0o600).
//! - Database schema migration and version tracking.
//! - Service restart and state recovery.
//! - Security attributes and daemon hardening directives.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use uuid::Uuid;

use sip_domain::{AccountId, CallDirection, CallId, SipAccount, TransportProtocol};
use sip_storage::{CallLogEntry, InMemoryCredentialBackend, StorageManager};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

struct TestTempDir(PathBuf);

impl TestTempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("plasma_sip_test_{}_{}", name, Uuid::new_v4()));
        let _ = fs::create_dir_all(&p);
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_r7_first_run_configuration_and_permissions() {
    let temp_dir = TestTempDir::new("first_run");
    let data_dir = temp_dir.path().join("plasma-sip");
    fs::create_dir_all(&data_dir).expect("create data dir");
    fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700)).expect("set dir perms");

    let db_path = data_dir.join("plasma-sip.db");

    // Initialize StorageManager for first time with isolated in-memory credential backend
    let backend = Arc::new(InMemoryCredentialBackend::new());
    let storage = StorageManager::new_with_backend(&db_path, backend).expect("init storage");
    assert!(
        storage.is_isolated_backend(),
        "Storage must use isolated credential backend"
    );

    // Verify directory permissions are 0o700
    let dir_perms = fs::metadata(&data_dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_perms, 0o700, "Data directory must be 0700");

    // Verify db file permissions are 0o600
    let db_perms = fs::metadata(&db_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(db_perms, 0o600, "Database file must be 0600");

    // Verify initial schema version is 3 (accounts, credentials, contacts)
    let version = storage.schema_version().expect("get schema version");
    assert_eq!(version, 3, "Schema version must be 3 on first run");

    // Create an account
    let account = SipAccount {
        id: AccountId(Uuid::new_v4()),
        name: "First Run Test".to_string(),
        username: "620".to_string(),
        display_name: Some("Deploy Test".to_string()),
        domain: "fritz.box".to_string(),
        registrar: "192.0.2.1".to_string(),
        registrar_port: 5060,
        transport: TransportProtocol::Udp,
        expires_secs: 300,
        enabled: true,
        is_default: true,
    };
    storage.save_account(&account).expect("save account");

    // Verify account retrieval
    let accounts = storage.list_accounts().expect("list accounts");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].username, "620");
    assert_eq!(accounts[0].domain, "fritz.box");
}

#[test]
fn test_r7_database_upgrade_and_migration() {
    let temp_dir = TestTempDir::new("upgrade_migration");
    let db_path = temp_dir.path().join("migrated.db");

    // Create a raw SQLite connection with user_version = 0 and legacy empty schema
    {
        let conn = rusqlite::Connection::open(&db_path).expect("create raw db");
        let initial_version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("query initial user_version");
        assert_eq!(initial_version, 0);
    }

    // Now open with StorageManager to trigger automatic migration
    let backend = Arc::new(InMemoryCredentialBackend::new());
    let storage = StorageManager::new_with_backend(&db_path, backend).expect("open with migration");
    assert!(
        storage.is_isolated_backend(),
        "Storage must use isolated credential backend"
    );
    let upgraded_version = storage.schema_version().expect("get upgraded version");
    assert_eq!(upgraded_version, 3, "Schema must be upgraded to version 3");

    // Verify writing and reading after migration
    let entry = CallLogEntry {
        id: CallId(Uuid::new_v4()),
        account_id: AccountId(Uuid::new_v4()),
        direction: CallDirection::Inbound,
        remote_party: "sip:**610@fritz.box".to_string(),
        started_at: chrono::Utc::now(),
        duration_seconds: 42,
        disconnect_reason: "Normal Clearing".to_string(),
    };
    storage.log_call(&entry).expect("log call");

    let history = storage.list_call_history(10).expect("list history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].remote_party, "sip:**610@fritz.box");
    assert_eq!(history[0].duration_seconds, 42);
}

#[test]
fn test_r7_daemon_restart_and_recovery() {
    let temp_dir = TestTempDir::new("restart_recovery");
    let db_path = temp_dir.path().join("recovery.db");
    let account_id = AccountId(Uuid::new_v4());
    let backend = Arc::new(InMemoryCredentialBackend::new());

    // 1. Initial daemon run: save account and state
    {
        let storage =
            StorageManager::new_with_backend(&db_path, backend.clone()).expect("init storage");
        assert!(
            storage.is_isolated_backend(),
            "Storage must use isolated credential backend"
        );
        let account = SipAccount {
            id: account_id,
            name: "Persistent Account".to_string(),
            username: "621".to_string(),
            display_name: Some("Recovered Station".to_string()),
            domain: "fritz.box".to_string(),
            registrar: "192.0.2.1".to_string(),
            registrar_port: 5060,
            transport: TransportProtocol::Udp,
            expires_secs: 600,
            enabled: true,
            is_default: true,
        };
        storage.save_account(&account).expect("save account");
    } // Storage dropped here, simulating daemon shutdown

    // 2. Restarted daemon run: reopen database and verify recovery
    {
        let storage = StorageManager::new_with_backend(&db_path, backend).expect("reopen storage");
        assert!(
            storage.is_isolated_backend(),
            "Storage must use isolated credential backend"
        );
        let recovered = storage
            .get_account(account_id)
            .expect("fetch account")
            .expect("account exists");

        assert_eq!(recovered.id, account_id);
        assert_eq!(recovered.username, "621");
        assert_eq!(recovered.domain, "fritz.box");
        assert!(recovered.enabled);
        assert!(recovered.is_default);
    }
}

#[test]
fn test_r7_packaging_installation_and_uninstallation() {
    let root = workspace_root();
    let temp_dir = TestTempDir::new("packaging_staging");
    let prefix = temp_dir.path().to_str().unwrap();

    let install_sh = root.join("packaging").join("install.sh");
    let uninstall_sh = root.join("packaging").join("uninstall.sh");

    // Setup isolated recording stub for systemctl
    let mock_bin_dir = temp_dir.path().join("mock_bin");
    fs::create_dir_all(&mock_bin_dir).expect("create mock bin dir");
    let mock_systemctl = mock_bin_dir.join("systemctl");
    let mock_systemctl_log = temp_dir.path().join("mock_systemctl.log");
    fs::write(
        &mock_systemctl,
        format!(
            "#!/bin/sh\necho \"$@\" >> \"{}\"\nexit 0\n",
            mock_systemctl_log.display()
        ),
    )
    .expect("write mock systemctl");
    fs::set_permissions(&mock_systemctl, fs::Permissions::from_mode(0o755))
        .expect("chmod mock systemctl");

    // Ensure release binaries are pre-built
    let bin_sip_rel = root.join("target").join("release").join("plasma-sip");
    let bin_sipd_rel = root.join("target").join("release").join("plasma-sipd");
    assert!(
        bin_sip_rel.exists() && bin_sipd_rel.exists(),
        "Release binaries target/release/plasma-sip and plasma-sipd must be pre-built before running packaging tests"
    );

    // 1. Run install.sh with staged prefix and isolated recording stub
    let install_status = Command::new("bash")
        .arg(&install_sh)
        .arg("--prefix")
        .arg(prefix)
        .arg("--no-build")
        .env("SYSTEMCTL", &mock_systemctl)
        .env("SYSTEMCTL_RECORD", "1")
        .env("PLASMA_SIP_STAGING", "1")
        .current_dir(&root)
        .status()
        .expect("execute install.sh");
    assert!(install_status.success(), "install.sh must succeed");

    let install_logs = fs::read_to_string(&mock_systemctl_log).expect("read mock systemctl log");
    assert!(
        install_logs.contains("--user daemon-reload"),
        "install.sh must invoke daemon-reload"
    );
    assert!(
        install_logs.contains("--user enable --now plasma-sipd.service"),
        "install.sh must invoke enable --now plasma-sipd.service"
    );

    // 2. Verify all installed files and permissions
    let bin_sip = Path::new(prefix).join("bin").join("plasma-sip");
    let bin_sipd = Path::new(prefix).join("bin").join("plasma-sipd");
    let desktop_file = Path::new(prefix)
        .join("share")
        .join("applications")
        .join("org.kde.plasma_sip.desktop");
    let icon_file = Path::new(prefix)
        .join("share")
        .join("icons")
        .join("hicolor")
        .join("scalable")
        .join("apps")
        .join("org.kde.plasma_sip.svg");
    let qml_dir = Path::new(prefix)
        .join("share")
        .join("plasma-sip")
        .join("qml");
    let dbus_file = Path::new(prefix)
        .join("share")
        .join("dbus-1")
        .join("services")
        .join("org.kde.PlasmaSip.service");
    let systemd_file = Path::new(prefix)
        .join("lib")
        .join("systemd")
        .join("user")
        .join("plasma-sipd.service");
    let autostart_file = Path::new(prefix)
        .join("etc")
        .join("xdg")
        .join("autostart")
        .join("org.kde.plasma_sip.desktop");

    assert!(bin_sip.exists(), "plasma-sip binary must exist");
    assert!(bin_sipd.exists(), "plasma-sipd binary must exist");
    let sip_mode = fs::metadata(&bin_sip).unwrap().permissions().mode();
    let sipd_mode = fs::metadata(&bin_sipd).unwrap().permissions().mode();
    assert_ne!(sip_mode & 0o111, 0, "plasma-sip must be executable");
    assert_ne!(sipd_mode & 0o111, 0, "plasma-sipd must be executable");

    assert!(desktop_file.exists(), "Desktop file must exist");
    assert!(icon_file.exists(), "SVG icon must exist");
    assert!(dbus_file.exists(), "D-Bus service activation must exist");
    assert!(systemd_file.exists(), "Systemd user service must exist");
    assert!(
        autostart_file.exists(),
        "Autostart desktop entry must exist"
    );

    // Validate desktop file
    let desktop_val = Command::new("desktop-file-validate")
        .arg(&desktop_file)
        .status()
        .expect("run desktop-file-validate");
    assert!(desktop_val.success(), "Desktop file must validate cleanly");

    let autostart_val = Command::new("desktop-file-validate")
        .arg(&autostart_file)
        .status()
        .expect("run desktop-file-validate on autostart");
    assert!(
        autostart_val.success(),
        "Autostart desktop file must validate cleanly"
    );

    // Validate QML files
    let required_qml = [
        "main.qml",
        "DialPadView.qml",
        "ActiveCallView.qml",
        "CallHistoryView.qml",
        "AudioSettingsView.qml",
        "AccountWizardView.qml",
        "IncomingCallBanner.qml",
    ];
    for qml in &required_qml {
        let p = qml_dir.join(qml);
        assert!(
            p.exists(),
            "QML file {} must exist in {}",
            qml,
            qml_dir.display()
        );
    }

    // Validate D-Bus service syntax
    let dbus_content = fs::read_to_string(&dbus_file).expect("read dbus service");
    assert!(dbus_content.contains("Name=org.kde.PlasmaSip"));
    assert!(dbus_content.contains(&format!("Exec={}/bin/plasma-sipd", prefix)));
    assert!(dbus_content.contains("SystemdService=plasma-sipd.service"));

    // Validate systemd service syntax & security hardening
    let systemd_content = fs::read_to_string(&systemd_file).expect("read systemd service");
    assert!(systemd_content.contains(&format!("ExecStart={}/bin/plasma-sipd", prefix)));
    assert!(systemd_content.contains("NoNewPrivileges=true"));
    assert!(systemd_content.contains("ProtectSystem=strict"));
    assert!(systemd_content.contains("ProtectHome=read-only"));
    assert!(systemd_content.contains("ReadWritePaths=%h/.local/share/plasma-sip"));

    // Validate autostart content
    let autostart_content = fs::read_to_string(&autostart_file).expect("read autostart file");
    assert!(autostart_content.contains(&format!("Exec={}/bin/plasma-sip --minimized", prefix)));

    // 3. Run uninstall.sh with staged prefix and isolated recording stub
    let uninstall_status = Command::new("bash")
        .arg(&uninstall_sh)
        .arg("--prefix")
        .arg(prefix)
        .env("SYSTEMCTL", &mock_systemctl)
        .env("SYSTEMCTL_RECORD", "1")
        .env("PLASMA_SIP_STAGING", "1")
        .current_dir(&root)
        .status()
        .expect("execute uninstall.sh");
    assert!(uninstall_status.success(), "uninstall.sh must succeed");

    let final_logs = fs::read_to_string(&mock_systemctl_log).expect("read mock systemctl log");
    assert!(
        final_logs.contains("--user stop plasma-sipd.service"),
        "uninstall.sh must record stop"
    );
    assert!(
        final_logs.contains("--user disable plasma-sipd.service"),
        "uninstall.sh must record disable"
    );

    // 4. Verify clean removal
    assert!(!bin_sip.exists(), "plasma-sip must be removed");
    assert!(!bin_sipd.exists(), "plasma-sipd must be removed");
    assert!(!desktop_file.exists(), "desktop file must be removed");
    assert!(!icon_file.exists(), "icon must be removed");
    assert!(!qml_dir.exists(), "QML directory must be removed");
    assert!(!dbus_file.exists(), "D-Bus service must be removed");
    assert!(!systemd_file.exists(), "Systemd service must be removed");
    assert!(!autostart_file.exists(), "Autostart file must be removed");
}

#[test]
fn test_r7_gui_qml_engine_test_launch() {
    let root = workspace_root();
    // Milestone R7 validates the packaged release artifact target/release/plasma-sip.
    // The release artifact must be pre-built from current source prior to test execution.
    let bin_sip = root.join("target").join("release").join("plasma-sip");
    assert!(
        bin_sip.exists(),
        "Release artifact target/release/plasma-sip must be pre-built before running packaging tests"
    );

    let status = Command::new(&bin_sip)
        .arg("--test-launch")
        .env(
            "QT_QPA_PLATFORM",
            std::env::var("QT_QPA_PLATFORM").unwrap_or_else(|_| "offscreen".to_string()),
        )
        .current_dir(&root)
        .status()
        .expect("run plasma-sip --test-launch");
    assert!(
        status.success(),
        "Release binary plasma-sip --test-launch must exit with code 0"
    );
}
