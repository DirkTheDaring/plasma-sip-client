import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    title: qsTr("SIP Accounts & PBX")

    property var bridge: null
    property var accountsList: []

    function updateAccountsList() {
        if (!page.bridge || !page.bridge.accounts_json) {
            accountsList = [];
            return;
        }
        try {
            accountsList = JSON.parse(page.bridge.accounts_json);
        } catch (e) {
            accountsList = [];
        }
    }

    Connections {
        target: page.bridge
        function onAccounts_jsonChanged() {
            page.updateAccountsList();
        }
    }

    onBridgeChanged: page.updateAccountsList()
    Component.onCompleted: {
        page.updateAccountsList();
        if (Qt.application.arguments.indexOf("--test-launch") !== -1) {
            accountSheet.open();
            accountSheet.close();
        }
    }

    function openNewAccount() {
        providerSelector.currentIndex = 0;
        hostInput.text = "fritz.box";
        usernameInput.text = "";
        passwordInput.text = "";
        displayNameInput.text = "";
        accountSheet.open();
    }

    function openEditAccount(acc) {
        var isFritz = (acc.registrar === "fritz.box" || acc.domain === "fritz.box");
        providerSelector.currentIndex = isFritz ? 0 : 1;
        hostInput.text = acc.registrar || acc.domain || "";
        usernameInput.text = acc.username || "";
        passwordInput.text = "";
        displayNameInput.text = acc.name || "";
        accountSheet.open();
    }

    actions: [
        Kirigami.Action {
            text: qsTr("Reconnect")
            icon.name: "network-connect"
            checkable: false
            enabled: page.accountsList.length > 0
            onTriggered: {
                if (page.bridge) page.bridge.reconnect();
            }
        },
        Kirigami.Action {
            text: qsTr("Add Account")
            icon.name: "list-add"
            checkable: false
            onTriggered: page.openNewAccount()
        },
        Kirigami.Action {
            text: qsTr("Refresh")
            icon.name: "view-refresh"
            checkable: false
            onTriggered: {
                if (page.bridge) page.bridge.refresh_accounts();
            }
        }
    ]

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            text: qsTr("Configured for AVM FRITZ!Box and generic SIP providers. Passwords are securely stored in the system keyring.")
            visible: true
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            visible: page.accountsList.length === 0
            text: qsTr("No SIP Accounts Configured")
            explanation: qsTr("Click 'Add Account' to configure an AVM FRITZ!Box or SIP provider account.")
            icon.name: "network-server"
        }

        // Configured Accounts Card List
        Kirigami.CardsLayout {
            Layout.fillWidth: true
            visible: page.accountsList.length > 0

            Repeater {
                model: page.accountsList

                Kirigami.Card {
                    banner.title: (modelData.name || qsTr("SIP Account")) + " (" + modelData.username + ")"
                    headerOrientation: Qt.Horizontal

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        RowLayout {
                            Layout.fillWidth: true
                            Label {
                                text: qsTr("Registrar:")
                                color: Kirigami.Theme.disabledTextColor
                            }
                            Label {
                                text: (modelData.registrar || modelData.domain || "fritz.box") + ":" + (modelData.registrar_port || 5060) + " (UDP)"
                                font.bold: true
                            }
                            Item { Layout.fillWidth: true }
                            Label {
                                text: modelData.enabled ? qsTr("Active") : qsTr("Disabled")
                                color: modelData.enabled ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                                font.bold: true
                            }
                        }

                        RowLayout {
                            spacing: Kirigami.Units.smallSpacing
                            Rectangle {
                                width: 10
                                height: 10
                                radius: 5
                                color: !modelData.enabled ? Kirigami.Theme.disabledTextColor :
                                       (page.bridge && page.bridge.is_registered ? Kirigami.Theme.positiveTextColor :
                                        (page.bridge && page.bridge.account_summary.indexOf("Failed") !== -1 ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.neutralTextColor))
                            }
                            Label {
                                text: !modelData.enabled ? qsTr("Disabled") :
                                      (page.bridge && page.bridge.is_registered ? qsTr("Registered (RFC 3261)") :
                                       (page.bridge ? page.bridge.account_summary : qsTr("Registering...")))
                                color: !modelData.enabled ? Kirigami.Theme.disabledTextColor :
                                       (page.bridge && page.bridge.is_registered ? Kirigami.Theme.positiveTextColor :
                                        (page.bridge && page.bridge.account_summary.indexOf("Failed") !== -1 ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.neutralTextColor))
                                font.pointSize: 10
                            }
                        }
                    }

                    actions: [
                        Kirigami.Action {
                            text: qsTr("Reconnect")
                            icon.name: "network-connect"
                            checkable: false
                            onTriggered: {
                                if (page.bridge && modelData.id) {
                                    page.bridge.reconnect_account(modelData.id);
                                }
                            }
                        },
                        Kirigami.Action {
                            text: qsTr("Edit")
                            icon.name: "document-edit"
                            checkable: false
                            onTriggered: {
                                page.openEditAccount(modelData);
                            }
                        },
                        Kirigami.Action {
                            text: qsTr("Remove")
                            icon.name: "edit-delete"
                            checkable: false
                            onTriggered: {
                                if (page.bridge && modelData.id) {
                                    page.bridge.delete_account(modelData.id);
                                }
                            }
                        }
                    ]
                }
            }
        }
    }

    // Modal Sheet for Account Creation Wizard
    Kirigami.OverlaySheet {
        id: accountSheet
        title: qsTr("Configure SIP Account")
        parent: applicationWindow() ? applicationWindow().overlay : null
        implicitHeight: Math.min(
            parent ? parent.height - Kirigami.Units.gridUnit * 4 : 500,
            accountFormLayout.implicitHeight + Kirigami.Units.gridUnit * 5
        )

        ColumnLayout {
            id: accountFormLayout
            spacing: Kirigami.Units.mediumSpacing
            implicitWidth: Kirigami.Units.gridUnit * 22

            ComboBox {
                id: providerSelector
                Layout.fillWidth: true
                model: [qsTr("AVM FRITZ!Box"), qsTr("Generic SIP Provider")]
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                TextField {
                    id: hostInput
                    Kirigami.FormData.label: qsTr("Host / Registrar:")
                    text: providerSelector.currentIndex === 0 ? "fritz.box" : ""
                    placeholderText: "fritz.box or sip.example.com"
                    Layout.fillWidth: true
                }

                TextField {
                    id: usernameInput
                    Kirigami.FormData.label: qsTr("SIP Username:")
                    placeholderText: providerSelector.currentIndex === 0 ? "620" : "username"
                    Layout.fillWidth: true
                }

                RowLayout {
                    Kirigami.FormData.label: qsTr("SIP Password:")
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    TextField {
                        id: passwordInput
                        Layout.fillWidth: true
                        echoMode: showPasswordBtn.checked ? TextInput.Normal : TextInput.Password
                        placeholderText: "••••••••"
                    }

                    ToolButton {
                        id: showPasswordBtn
                        checkable: true
                        icon.name: checked ? "view-hidden" : "view-visible"
                        ToolTip.text: checked ? qsTr("Hide password") : qsTr("Show password")
                        ToolTip.visible: hovered
                        focusPolicy: Qt.NoFocus
                    }
                }

                TextField {
                    id: displayNameInput
                    Kirigami.FormData.label: qsTr("Display Name:")
                    placeholderText: qsTr("Plasma Softphone")
                    Layout.fillWidth: true
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button {
                    text: qsTr("Cancel")
                    onClicked: accountSheet.close()
                }
                Button {
                    text: qsTr("Save & Connect")
                    icon.name: "dialog-ok"
                    enabled: usernameInput.text.trim().length > 0 && passwordInput.text.trim().length > 0
                    onClicked: {
                        if (page.bridge) {
                            var host = hostInput.text.trim() || (providerSelector.currentIndex === 0 ? "fritz.box" : "");
                            var name = displayNameInput.text.trim() || (providerSelector.currentIndex === 0 ? "FRITZ!Box Telephony" : "SIP Account");
                            page.bridge.save_account(
                                name,
                                usernameInput.text.trim(),
                                passwordInput.text.trim(),
                                host,
                                host,
                                5060
                            );
                        }
                        accountSheet.close();
                    }
                }
            }
        }
    }
}
