import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma_sip

Kirigami.ApplicationWindow {
    id: root
    title: qsTr("Plasma SIP")
    width: 480
    height: 720
    minimumWidth: 360
    minimumHeight: 600
    visible: Qt.application.arguments.indexOf("--minimized") === -1 && Qt.application.arguments.indexOf("--tray") === -1
    globalDrawer: null
    contextDrawer: null

    // Close window to system tray
    onClosing: function(close) {
        close.accepted = false;
        root.hide();
    }

    function showAndRaise() {
        root.visible = true;
        root.showNormal();
        root.show();
        root.raise();
        root.requestActivate();
    }

    function toggleWindow() {
        if (root.visible && root.active && root.visibility !== Window.Minimized) {
            root.hide();
        } else {
            showAndRaise();
        }
    }

    function switchToPage(targetPage) {
        if (!targetPage || root.pageStack.currentItem === targetPage) {
            return;
        }
        root.pageStack.replace(targetPage);
    }

    // CXX-Qt Rust / D-Bus Bridge
    SipUiBridge {
        id: sipBridge

        onRaise_window_tokenChanged: {
            root.showAndRaise();
            if (sipBridge.active_call_state === "Dialing" || sipBridge.active_call_state === "Early" || sipBridge.active_call_state === "Active") {
                root.switchToPage(activeCallView);
            }
        }

        onToggle_window_tokenChanged: {
            root.toggleWindow();
        }

        onPrefilled_dial_numberChanged: {
            if (sipBridge.prefilled_dial_number !== "") {
                dialPadView.prefillNumber(sipBridge.prefilled_dial_number);
                root.switchToPage(dialPadView);
                root.showAndRaise();
            }
        }

        onActive_call_stateChanged: {
            if (sipBridge.active_call_state === "Dialing" || sipBridge.active_call_state === "Ringing" || sipBridge.active_call_state === "Early" || sipBridge.active_call_state === "Active" || sipBridge.active_call_state === "Held") {
                root.switchToPage(activeCallView);
            } else if (sipBridge.active_call_state === "Idle" || sipBridge.active_call_state === "Terminated") {
                if (root.pageStack.currentItem === activeCallView) {
                    root.switchToPage(dialPadView);
                }
            }
        }
    }

    pageStack.initialPage: dialPadView

    Component.onCompleted: {
        if (Qt.application.arguments.indexOf("--test-launch") !== -1) {
            root.switchToPage(dialPadView);
            root.switchToPage(historyView);
            root.switchToPage(contactsView);
            root.switchToPage(settingsView);
            root.switchToPage(dialPadView);
        }
    }

    // Direct Access Bottom Navigation Bar (HIG Convergent Softphone Standard: 4 Tabs)
    footer: TabBar {
        id: navTabBar
        visible: sipBridge.active_call_state !== "Active" && sipBridge.active_call_state !== "Dialing" && sipBridge.active_call_state !== "Early" && sipBridge.active_call_state !== "Ringing" && sipBridge.active_call_state !== "Held"
        currentIndex: root.pageStack.currentItem === dialPadView ? 0 :
                      (root.pageStack.currentItem === historyView ? 1 :
                       (root.pageStack.currentItem === contactsView ? 2 :
                        (root.pageStack.currentItem === settingsView ? 3 : 0)))

        TabButton {
            text: qsTr("Keypad")
            icon.name: "call-start"
            onClicked: root.switchToPage(dialPadView)
        }
        TabButton {
            text: qsTr("History")
            icon.name: "view-history"
            onClicked: {
                sipBridge.refresh_history();
                root.switchToPage(historyView);
            }
        }
        TabButton {
            text: qsTr("Contacts")
            icon.name: "address-book-new"
            onClicked: {
                sipBridge.refresh_contacts();
                root.switchToPage(contactsView);
            }
        }
        TabButton {
            text: qsTr("Settings")
            icon.name: "settings-configure"
            onClicked: {
                sipBridge.refresh_accounts();
                sipBridge.refresh_devices();
                root.switchToPage(settingsView);
            }
        }
    }

    // Global Fast Navigation Shortcuts
    Shortcut {
        sequence: "Ctrl+1"
        onActivated: root.switchToPage(dialPadView)
    }
    Shortcut {
        sequence: "Ctrl+2"
        onActivated: {
            sipBridge.refresh_history();
            root.switchToPage(historyView);
        }
    }
    Shortcut {
        sequence: "Ctrl+3"
        onActivated: {
            sipBridge.refresh_contacts();
            root.switchToPage(contactsView);
        }
    }
    Shortcut {
        sequence: "Ctrl+4"
        onActivated: {
            sipBridge.refresh_accounts();
            sipBridge.refresh_devices();
            root.switchToPage(settingsView);
        }
    }
    Shortcut {
        sequence: "Ctrl+W"
        onActivated: root.hide()
    }
    Shortcut {
        sequence: "Ctrl+Q"
        onActivated: {
            sipBridge.quit_app();
            Qt.quit();
        }
    }

    DialPadView {
        id: dialPadView
        visible: false
        bridge: sipBridge
        onStartCallRequested: function(target) {
            sipBridge.dial(target);
        }
        onOpenAccountsRequested: {
            sipBridge.refresh_accounts();
            root.switchToPage(settingsView);
        }
    }

    ActiveCallView {
        id: activeCallView
        visible: false
        bridge: sipBridge
        remoteParty: sipBridge.active_remote_party
        callState: sipBridge.active_call_state
        durationSeconds: sipBridge.call_duration_seconds
        isMuted: sipBridge.is_muted
        onHangupRequested: {
            sipBridge.hangup_call();
            root.switchToPage(dialPadView);
        }
        onToggleMuteRequested: {
            sipBridge.toggle_mute();
        }
        onHoldRequested: {
            sipBridge.hold_call();
        }
        onResumeRequested: {
            sipBridge.resume_call();
        }
        onSendDtmfRequested: function(digit) {
            sipBridge.send_dtmf(digit);
        }
    }

    CallHistoryView {
        id: historyView
        visible: false
        bridge: sipBridge
        onRedialRequested: function(target) {
            sipBridge.dial(target);
        }
    }

    ContactsView {
        id: contactsView
        visible: false
        bridge: sipBridge
        onStartCallRequested: function(target) {
            sipBridge.dial(target);
        }
        onPrefillRequested: function(target) {
            dialPadView.prefillNumber(target);
            root.switchToPage(dialPadView);
        }
    }

    SettingsView {
        id: settingsView
        visible: false
        bridge: sipBridge
    }

    // Incoming Call Modal / Overlay
    IncomingCallBanner {
        id: incomingBanner
        anchors.fill: parent
        visible: sipBridge.incoming_call_id !== ""
        caller: sipBridge.incoming_caller
        isSilent: {
            if (!sipBridge.audio_config_json) return false;
            try {
                var parsed = JSON.parse(sipBridge.audio_config_json);
                return parsed && parsed.silent_mode === true;
            } catch (e) {
                return false;
            }
        }
        onAccepted: {
            sipBridge.answer_call();
            root.switchToPage(activeCallView);
        }
        onRejected: {
            sipBridge.reject_call();
        }
    }
}
