import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Contacts")

    property var bridge: null
    signal startCallRequested(string target)
    signal prefillRequested(string target)

    // Debounce timer for search queries
    Timer {
        id: searchDebounceTimer
        interval: 150
        repeat: false
        onTriggered: {
            if (page.bridge) {
                page.bridge.search_contacts(searchField.text.trim());
            }
        }
    }

    // Parsed contacts list from D-Bus JSON
    readonly property var rawContactsList: {
        if (!page.bridge || !page.bridge.contacts_json) return [];
        try {
            var raw = page.bridge.contacts_json;
            if (!raw || raw === "" || raw === "[]") return [];
            return JSON.parse(raw);
        } catch (e) {
            return [];
        }
    }

    // Helper: deterministic pastel avatar background color based on name
    function getAvatarColor(name) {
        if (!name || name.length === 0) return Kirigami.Theme.highlightColor;
        var hash = 0;
        for (var i = 0; i < name.length; i++) {
            hash = name.charCodeAt(i) + ((hash << 5) - hash);
        }
        var hues = [200, 220, 160, 280, 30, 340, 180, 260];
        var hue = hues[Math.abs(hash) % hues.length];
        return Qt.hsla(hue / 360.0, 0.65, 0.45, 1.0);
    }

    // Helper: extract 1-2 uppercase initials from name
    function getInitials(name) {
        if (!name) return "?";
        var parts = name.trim().split(/\s+/);
        if (parts.length >= 2 && parts[0].length > 0 && parts[1].length > 0) {
            return (parts[0][0] + parts[1][0]).toUpperCase();
        } else if (parts.length === 1 && parts[0].length > 0) {
            return parts[0].substring(0, Math.min(2, parts[0].length)).toUpperCase();
        }
        return "?";
    }

    // Helper: readable label for number type
    function getNumberTypeLabel(typeStr) {
        switch (typeStr) {
            case "home": return qsTr("Home");
            case "mobile": return qsTr("Mobile");
            case "work": return qsTr("Work");
            case "internal": return qsTr("Internal");
            case "quickdial": return qsTr("Quickdial");
            case "vanity": return qsTr("Vanity");
            default: return qsTr("Other");
        }
    }

    // Helper: icon name for number type
    function getNumberTypeIcon(typeStr) {
        switch (typeStr) {
            case "home": return "user-home";
            case "mobile": return "phone";
            case "work": return "view-institution";
            case "internal": return "network-server";
            case "quickdial": return "favorites";
            default: return "call-start";
        }
    }

    // Header actions
    actions: [
        Kirigami.Action {
            text: qsTr("Sync FRITZ!Box")
            icon.name: "view-refresh"
            tooltip: qsTr("Synchronize phonebook from FRITZ!Box or PBX via TR-064")
            enabled: page.bridge && !page.bridge.is_syncing_contacts
            onTriggered: {
                if (page.bridge) {
                    page.bridge.sync_phonebook();
                }
            }
        },
        Kirigami.Action {
            text: qsTr("Phonebook Settings")
            icon.name: "configure"
            tooltip: qsTr("Configure FRITZ!Box router credentials & phonebook ID")
            onTriggered: {
                if (page.bridge) page.bridge.refresh_phonebook_config();
                phonebookSetupSheet.open();
            }
        }
    ]

    Component.onCompleted: {
        if (Qt.application.arguments.indexOf("--test-launch") !== -1) {
            phonebookSetupSheet.open();
            phonebookSetupSheet.close();
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Kirigami.Units.smallSpacing

        // Search Bar & Filter Header
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            Kirigami.SearchField {
                id: searchField
                Layout.fillWidth: true
                placeholderText: qsTr("Search contacts or dial number (T9 supported)...")
                onTextChanged: {
                    searchDebounceTimer.restart();
                }
            }

            ToolButton {
                icon.name: "view-refresh"
                text: qsTr("Sync")
                display: AbstractButton.IconOnly
                ToolTip.text: qsTr("Synchronize phonebook from FRITZ!Box (TR-064)")
                ToolTip.visible: hovered
                enabled: page.bridge && !page.bridge.is_syncing_contacts
                onClicked: {
                    if (page.bridge) page.bridge.sync_phonebook();
                }
            }

            ToolButton {
                icon.name: "configure"
                text: qsTr("Settings")
                display: AbstractButton.IconOnly
                ToolTip.text: qsTr("Configure FRITZ!Box Phonebook Credentials")
                ToolTip.visible: hovered
                onClicked: {
                    if (page.bridge) page.bridge.refresh_phonebook_config();
                    phonebookSetupSheet.open();
                }
            }
        }

        // Sync in Progress or Status Notice
        Rectangle {
            id: syncStatusNotice
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.smallSpacing
            Layout.rightMargin: Kirigami.Units.smallSpacing
            implicitHeight: syncRow.implicitHeight + 12
            radius: 8
            visible: (page.bridge && page.bridge.is_syncing_contacts) || (page.bridge && page.bridge.contact_sync_status !== "")
            color: page.bridge && page.bridge.is_syncing_contacts ?
                   Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15) :
                   (page.bridge && page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1 ?
                    Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.15) :
                    Qt.rgba(Kirigami.Theme.positiveTextColor.r, Kirigami.Theme.positiveTextColor.g, Kirigami.Theme.positiveTextColor.b, 0.15))
            border.color: page.bridge && page.bridge.is_syncing_contacts ?
                          Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.3) :
                          (page.bridge && page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1 ?
                           Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.3) :
                           Qt.rgba(Kirigami.Theme.positiveTextColor.r, Kirigami.Theme.positiveTextColor.g, Kirigami.Theme.positiveTextColor.b, 0.3))
            border.width: 1

            RowLayout {
                id: syncRow
                anchors.fill: parent
                anchors.margins: 6
                spacing: Kirigami.Units.smallSpacing

                BusyIndicator {
                    running: page.bridge && page.bridge.is_syncing_contacts
                    visible: running
                    Layout.preferredWidth: 20
                    Layout.preferredHeight: 20
                }

                Kirigami.Icon {
                    source: page.bridge && (page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1) ?
                            "dialog-warning" : "dialog-information"
                    visible: page.bridge && !page.bridge.is_syncing_contacts
                    implicitWidth: 16
                    implicitHeight: 16
                    color: page.bridge && (page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1) ?
                           Kirigami.Theme.negativeTextColor : Kirigami.Theme.positiveTextColor
                }

                Label {
                    Layout.fillWidth: true
                    text: page.bridge ? page.bridge.contact_sync_status : ""
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                    elide: Text.ElideRight
                    color: Kirigami.Theme.textColor
                }

                ToolButton {
                    icon.name: "dialog-close"
                    display: AbstractButton.IconOnly
                    visible: page.bridge && !page.bridge.is_syncing_contacts && page.bridge.contact_sync_status !== ""
                    implicitWidth: 24
                    implicitHeight: 24
                    onClicked: {
                        if (page.bridge) page.bridge.contact_sync_status = "";
                    }
                }
            }
        }

        // Empty Placeholder
        Kirigami.PlaceholderMessage {
            id: emptyPlaceholder
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: page.rawContactsList.length === 0
            icon.name: page.bridge && !page.bridge.phonebook_has_password && searchField.text.trim().length === 0 ? "configure" : "address-book-new"
            text: searchField.text.trim().length > 0 ? qsTr("No matching contacts") :
                  (page.bridge && !page.bridge.phonebook_has_password ? qsTr("FRITZ!Box Phonebook Not Configured") : qsTr("No Contacts Found"))
            explanation: searchField.text.trim().length > 0 ?
                         qsTr("Try searching with different terms or check spelling.") :
                         (page.bridge && !page.bridge.phonebook_has_password ?
                          qsTr("Configure your FRITZ!Box login credentials to synchronize contacts, internal extensions, and speed dials via TR-064.") :
                          qsTr("Synchronize your phonebook from your AVM FRITZ!Box or PBX to quickly search and dial."))

            helpfulAction: Kirigami.Action {
                text: (page.bridge && !page.bridge.phonebook_has_password && searchField.text.trim().length === 0) ?
                      qsTr("Set Up Credentials") : qsTr("Synchronize from FRITZ!Box")
                icon.name: (page.bridge && !page.bridge.phonebook_has_password && searchField.text.trim().length === 0) ?
                           "configure" : "view-refresh"
                visible: searchField.text.trim().length === 0
                enabled: page.bridge && !page.bridge.is_syncing_contacts
                onTriggered: {
                    if (page.bridge && !page.bridge.phonebook_has_password) {
                        page.bridge.refresh_phonebook_config();
                        phonebookSetupSheet.open();
                    } else if (page.bridge) {
                        page.bridge.sync_phonebook();
                    }
                }
            }
        }

        // Contacts List
        ListView {
            id: contactsListView
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: page.rawContactsList.length > 0
            model: page.rawContactsList
            clip: true
            spacing: Kirigami.Units.smallSpacing

            delegate: ItemDelegate {
                id: contactDelegate
                width: ListView.view ? ListView.view.width : parent.width
                hoverEnabled: true

                contentItem: RowLayout {
                    spacing: Kirigami.Units.mediumSpacing

                    // Initial Avatar Bubble
                    Rectangle {
                        implicitWidth: 44
                        implicitHeight: 44
                        radius: 22
                        color: page.getAvatarColor(modelData.name)

                        Label {
                            anchors.centerIn: parent
                            text: page.getInitials(modelData.name)
                            font.bold: true
                            font.pointSize: 13
                            color: "white"
                        }
                    }

                    // Contact Information & Phone Numbers
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 4

                        // Name and Badges Header
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.smallSpacing

                            Label {
                                text: modelData.name || qsTr("Unknown Contact")
                                font.weight: Font.DemiBold
                                font.pointSize: Kirigami.Theme.defaultFont.pointSize + 1
                                elide: Text.ElideRight
                                color: Kirigami.Theme.textColor
                            }

                            // Quickdial Tag
                            Rectangle {
                                visible: !!modelData.quickdial && modelData.quickdial !== ""
                                implicitHeight: 18
                                implicitWidth: qdLabel.implicitWidth + 10
                                radius: 9
                                color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                border.color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.3)
                                border.width: 1

                                Label {
                                    id: qdLabel
                                    anchors.centerIn: parent
                                    text: "**" + modelData.quickdial
                                    font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.85
                                    font.weight: Font.Medium
                                    color: Kirigami.Theme.highlightColor
                                }
                            }

                            // Source badge
                            Rectangle {
                                visible: modelData.source === "fritzbox"
                                implicitHeight: 18
                                implicitWidth: fbLabel.implicitWidth + 10
                                radius: 9
                                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)

                                Label {
                                    id: fbLabel
                                    anchors.centerIn: parent
                                    text: "FRITZ!Box"
                                    font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.8
                                    color: Kirigami.Theme.disabledTextColor
                                }
                            }

                            Item { Layout.fillWidth: true }
                        }

                        // Numbers List
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 4

                            Repeater {
                                model: modelData.numbers || []

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: Kirigami.Units.smallSpacing

                                    // Type badge
                                    Rectangle {
                                        implicitHeight: 20
                                        implicitWidth: typeRow.implicitWidth + 12
                                        radius: 10
                                        color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.06)

                                        RowLayout {
                                            id: typeRow
                                            anchors.centerIn: parent
                                            spacing: 4

                                            Kirigami.Icon {
                                                source: page.getNumberTypeIcon(modelData.number_type)
                                                implicitWidth: 10
                                                implicitHeight: 10
                                                color: Kirigami.Theme.disabledTextColor
                                            }

                                            Label {
                                                text: page.getNumberTypeLabel(modelData.number_type)
                                                font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.85
                                                color: Kirigami.Theme.disabledTextColor
                                            }
                                        }
                                    }

                                    // Phone Number String
                                    Label {
                                        Layout.fillWidth: true
                                        text: modelData.number
                                        font.pointSize: Kirigami.Theme.defaultFont.pointSize
                                        font.family: "Noto Sans, Inter, Roboto, sans-serif"
                                        elide: Text.ElideRight
                                        color: Kirigami.Theme.textColor
                                    }

                                    // Dial / Call Action Button
                                    RoundButton {
                                        id: callBtn
                                        implicitWidth: 32
                                        implicitHeight: 32
                                        icon.name: "call-start"
                                        icon.width: 16
                                        icon.height: 16
                                        ToolTip.text: qsTr("Call %1").arg(modelData.number)
                                        ToolTip.visible: hovered
                                        background: Rectangle {
                                            radius: 16
                                            color: callBtn.hovered ?
                                                   (callBtn.pressed ? Qt.darker("#27ae60", 1.2) : "#27ae60") :
                                                   Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.12)
                                        }
                                        onClicked: {
                                            page.startCallRequested(modelData.number);
                                        }
                                    }

                                    // Pre-fill into Keypad Button
                                    RoundButton {
                                        id: prefillBtn
                                        implicitWidth: 32
                                        implicitHeight: 32
                                        icon.name: "input-dialpad"
                                        icon.width: 14
                                        icon.height: 14
                                        ToolTip.text: qsTr("Open in keypad")
                                        ToolTip.visible: hovered
                                        background: Rectangle {
                                            radius: 16
                                            color: prefillBtn.hovered ?
                                                   Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.15) :
                                                   "transparent"
                                        }
                                        onClicked: {
                                            page.prefillRequested(modelData.number);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Modal Sheet for FRITZ!Box Phonebook Credentials & Setup
    Kirigami.OverlaySheet {
        id: phonebookSetupSheet
        title: qsTr("FRITZ!Box Phonebook Credentials")
        parent: applicationWindow() ? applicationWindow().overlay : null
        implicitHeight: Math.min(
            parent ? parent.height - Kirigami.Units.gridUnit * 4 : 560,
            phonebookSetupLayout.implicitHeight + Kirigami.Units.gridUnit * 5
        )

        onVisibleChanged: {
            if (visible && page.bridge) {
                fbHostField.text = page.bridge.phonebook_host || "fritz.box";
                fbUserField.text = page.bridge.phonebook_username || "";
                fbPasswordField.text = "";
                fbIdSpin.value = page.bridge.phonebook_id || 0;
                fbAutoSyncCheck.checked = page.bridge.phonebook_auto_sync;
            }
        }

        ColumnLayout {
            id: phonebookSetupLayout
            spacing: Kirigami.Units.mediumSpacing
            implicitWidth: Math.min(page.width - (Kirigami.Units.gridUnit * 2), Kirigami.Units.gridUnit * 24)

            // Info Card
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: infoRow.implicitHeight + 16
                radius: 8
                color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.08)
                border.color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.25)
                border.width: 1

                RowLayout {
                    id: infoRow
                    anchors.fill: parent
                    anchors.margins: 10
                    spacing: Kirigami.Units.mediumSpacing

                    Kirigami.Icon {
                        source: "security-high"
                        implicitWidth: 24
                        implicitHeight: 24
                        color: Kirigami.Theme.highlightColor
                    }

                    Label {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: qsTr("AVM FRITZ!Box phonebooks require your router web login (TR-064 on port 49000). Passwords are encrypted in your local KDE Wallet / Secret Service.")
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        color: Kirigami.Theme.textColor
                    }
                }
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                TextField {
                    id: fbHostField
                    Kirigami.FormData.label: qsTr("Router Host / IP:")
                    text: page.bridge ? page.bridge.phonebook_host : "fritz.box"
                    placeholderText: "fritz.box or 192.168.178.1"
                    Layout.fillWidth: true
                }

                TextField {
                    id: fbUserField
                    Kirigami.FormData.label: qsTr("Username:")
                    text: page.bridge ? page.bridge.phonebook_username : ""
                    placeholderText: "admin or FRITZ!Box user"
                    Layout.fillWidth: true
                }

                RowLayout {
                    Kirigami.FormData.label: qsTr("Password:")
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    TextField {
                        id: fbPasswordField
                        Layout.fillWidth: true
                        echoMode: showFbPwdBtn.checked ? TextInput.Normal : TextInput.Password
                        placeholderText: page.bridge && page.bridge.phonebook_has_password ?
                                         qsTr("Saved in KDE Wallet (leave blank to keep)") :
                                         qsTr("FRITZ!Box password")
                    }

                    ToolButton {
                        id: showFbPwdBtn
                        checkable: true
                        icon.name: checked ? "view-hidden" : "view-visible"
                        ToolTip.text: checked ? qsTr("Hide password") : qsTr("Show password")
                        ToolTip.visible: hovered
                        focusPolicy: Qt.NoFocus
                    }
                }

                RowLayout {
                    Kirigami.FormData.label: qsTr("Phonebook ID:")
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    SpinBox {
                        id: fbIdSpin
                        from: 0
                        to: 20
                        value: page.bridge ? page.bridge.phonebook_id : 0
                    }

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("(0 = Default / Main phonebook)")
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        color: Kirigami.Theme.disabledTextColor
                    }
                }

                CheckBox {
                    id: fbAutoSyncCheck
                    Kirigami.FormData.label: qsTr("Auto-Sync:")
                    text: qsTr("Synchronize automatically on application startup")
                    checked: page.bridge ? page.bridge.phonebook_auto_sync : false
                }
            }

            // Sync Status feedback
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: syncFeedbackRow.implicitHeight + 10
                radius: 6
                visible: (page.bridge && page.bridge.is_syncing_contacts) || (page.bridge && page.bridge.contact_sync_status !== "")
                color: page.bridge && page.bridge.is_syncing_contacts ?
                       Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.12) :
                       (page.bridge && (page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1) ?
                        Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.12) :
                        Qt.rgba(Kirigami.Theme.positiveTextColor.r, Kirigami.Theme.positiveTextColor.g, Kirigami.Theme.positiveTextColor.b, 0.12))

                RowLayout {
                    id: syncFeedbackRow
                    anchors.fill: parent
                    anchors.margins: 6
                    spacing: Kirigami.Units.smallSpacing

                    BusyIndicator {
                        running: page.bridge && page.bridge.is_syncing_contacts
                        visible: running
                        Layout.preferredWidth: 18
                        Layout.preferredHeight: 18
                    }

                    Kirigami.Icon {
                        source: page.bridge && (page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1) ?
                                "dialog-warning" : "dialog-information"
                        visible: page.bridge && !page.bridge.is_syncing_contacts
                        implicitWidth: 16
                        implicitHeight: 16
                        color: page.bridge && (page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1) ?
                               Kirigami.Theme.negativeTextColor : Kirigami.Theme.positiveTextColor
                    }

                    Label {
                        Layout.fillWidth: true
                        text: page.bridge ? page.bridge.contact_sync_status : ""
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        elide: Text.ElideRight
                        color: Kirigami.Theme.textColor
                    }
                }
            }

            // Buttons
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Button {
                    text: qsTr("Save & Sync Now")
                    icon.name: "view-refresh"
                    highlighted: true
                    enabled: page.bridge && !page.bridge.is_syncing_contacts
                    onClicked: {
                        if (page.bridge) {
                            page.bridge.save_phonebook_config(
                                fbHostField.text.trim(),
                                fbUserField.text.trim(),
                                fbPasswordField.text,
                                fbIdSpin.value,
                                fbAutoSyncCheck.checked
                            );
                            page.bridge.sync_phonebook_with_config(
                                fbHostField.text.trim(),
                                fbUserField.text.trim(),
                                fbPasswordField.text,
                                fbIdSpin.value,
                                true
                            );
                        }
                    }
                }

                Button {
                    text: qsTr("Save Only")
                    icon.name: "document-save"
                    onClicked: {
                        if (page.bridge) {
                            page.bridge.save_phonebook_config(
                                fbHostField.text.trim(),
                                fbUserField.text.trim(),
                                fbPasswordField.text,
                                fbIdSpin.value,
                                fbAutoSyncCheck.checked
                            );
                        }
                        phonebookSetupSheet.close();
                    }
                }

                Item { Layout.fillWidth: true }

                Button {
                    text: qsTr("Close")
                    icon.name: "dialog-close"
                    onClicked: phonebookSetupSheet.close()
                }
            }
        }
    }
}
