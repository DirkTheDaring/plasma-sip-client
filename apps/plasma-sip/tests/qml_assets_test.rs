use std::fs;
use std::path::Path;

#[test]
fn test_qml_interface_files_exist_and_valid() {
    let qml_dir = Path::new("../../qml");
    let qml_dir = if qml_dir.exists() {
        qml_dir
    } else {
        Path::new("qml")
    };
    assert!(qml_dir.is_dir(), "qml/ directory must exist");

    let required_files = [
        "main.qml",
        "DialPadView.qml",
        "ActiveCallView.qml",
        "IncomingCallBanner.qml",
        "AccountWizardView.qml",
        "CallHistoryView.qml",
        "AudioSettingsView.qml",
        "ContactsView.qml",
        "SettingsView.qml",
    ];

    for file_name in &required_files {
        let file_path = qml_dir.join(file_name);
        assert!(
            file_path.is_file(),
            "Missing required QML file: {}",
            file_name
        );

        let content = fs::read_to_string(&file_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", file_name));
        assert!(
            !content.trim().is_empty(),
            "QML file is empty: {}",
            file_name
        );

        // Verify Qt Quick & Kirigami imports
        assert!(
            content.contains("import QtQuick"),
            "{} must import QtQuick",
            file_name
        );
        assert!(
            content.contains("import org.kde.kirigami"),
            "{} must import KDE Kirigami",
            file_name
        );
    }
}

#[test]
fn test_dialpad_elements_and_fritzbox_extensions() {
    let path = if Path::new("../../qml/DialPadView.qml").exists() {
        "../../qml/DialPadView.qml"
    } else {
        "qml/DialPadView.qml"
    };
    let content = fs::read_to_string(path).expect("read DialPadView.qml");

    // Standard telephone digits
    for digit in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "*", "#"] {
        assert!(
            content.contains(&format!("\"{}\"", digit)),
            "DialPadView must contain key '{}'",
            digit
        );
    }

    // FRITZ!Box internal extensions
    assert!(
        content.contains("**610"),
        "DialPadView must contain quick extension **610"
    );
    assert!(
        content.contains("**611"),
        "DialPadView must contain quick extension **611"
    );
    assert!(
        content.contains("**9"),
        "DialPadView must contain quick broadcast extension **9"
    );
}

#[test]
fn test_active_call_view_controls() {
    let path = if Path::new("../../qml/ActiveCallView.qml").exists() {
        "../../qml/ActiveCallView.qml"
    } else {
        "qml/ActiveCallView.qml"
    };
    let content = fs::read_to_string(path).expect("read ActiveCallView.qml");

    assert!(
        content.contains("hangupRequested"),
        "Must provide hangupRequested signal"
    );
    assert!(
        content.contains("toggleMuteRequested"),
        "Must provide toggleMuteRequested signal"
    );
    assert!(
        content.contains("durationSeconds"),
        "Must display call duration"
    );
    assert!(
        content.contains("formatTime"),
        "Must format call duration timer"
    );
    assert!(
        content.contains("call-stop"),
        "Must provide end-call button"
    );
}

#[test]
fn test_incoming_call_banner_actions() {
    let path = if Path::new("../../qml/IncomingCallBanner.qml").exists() {
        "../../qml/IncomingCallBanner.qml"
    } else {
        "qml/IncomingCallBanner.qml"
    };
    let content = fs::read_to_string(path).expect("read IncomingCallBanner.qml");

    assert!(
        content.contains("signal accepted()"),
        "Must provide accepted signal"
    );
    assert!(
        content.contains("signal rejected()"),
        "Must provide rejected signal"
    );
    assert!(content.contains("call-start"), "Must provide accept button");
    assert!(content.contains("call-stop"), "Must provide decline button");
}

#[test]
fn test_account_wizard_fritzbox_and_generic() {
    let path = if Path::new("../../qml/AccountWizardView.qml").exists() {
        "../../qml/AccountWizardView.qml"
    } else {
        "qml/AccountWizardView.qml"
    };
    let content = fs::read_to_string(path).expect("read AccountWizardView.qml");

    assert!(
        content.contains("fritz.box"),
        "Must provide fritz.box suggested host"
    );
    assert!(
        content.contains("620"),
        "Must provide 620 suggested internal extension"
    );
    assert!(
        content.contains("AVM FRITZ!Box"),
        "Must support FRITZ!Box profile"
    );
    assert!(
        content.contains("Generic SIP Provider"),
        "Must support Generic SIP profile"
    );
    assert!(content.contains("Password"), "Must mask password input");
}

#[test]
fn test_contacts_view_elements_and_sync() {
    let path = if Path::new("../../qml/ContactsView.qml").exists() {
        "../../qml/ContactsView.qml"
    } else {
        "qml/ContactsView.qml"
    };
    let content = fs::read_to_string(path).expect("read ContactsView.qml");

    assert!(
        content.contains("sync_phonebook"),
        "Must provide sync_phonebook trigger"
    );
    assert!(
        content.contains("search_contacts"),
        "Must provide search_contacts hook"
    );
    assert!(
        content.contains("getAvatarColor"),
        "Must provide deterministic avatar color calculation"
    );
    assert!(
        content.contains("startCallRequested"),
        "Must provide startCallRequested signal"
    );
    assert!(
        content.contains("prefillRequested"),
        "Must provide prefillRequested signal"
    );
}

#[test]
fn test_settings_view_elements_and_sections() {
    let path = if Path::new("../../qml/SettingsView.qml").exists() {
        "../../qml/SettingsView.qml"
    } else {
        "qml/SettingsView.qml"
    };
    let content = fs::read_to_string(path).expect("read SettingsView.qml");

    assert!(
        content.contains("AccountWizardView"),
        "Must embed AccountWizardView"
    );
    assert!(
        content.contains("AudioSettingsView"),
        "Must embed AudioSettingsView"
    );
    assert!(
        content.contains("sync_phonebook"),
        "Must support phonebook sync action"
    );
    assert!(
        content.contains("change_reject_mode"),
        "Must support call reject mode configuration"
    );
}
