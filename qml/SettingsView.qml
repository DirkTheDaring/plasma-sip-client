import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Settings")

    property var bridge: null

    readonly property int contactsCount: {
        if (!page.bridge || !page.bridge.contacts_json) return 0;
        try {
            var parsed = JSON.parse(page.bridge.contacts_json);
            return Array.isArray(parsed) ? parsed.length : 0;
        } catch (e) {
            return 0;
        }
    }

    header: TabBar {
        id: settingsTabBar
        currentIndex: 0

        TabButton {
            text: qsTr("Accounts & PBX")
            icon.name: "network-server"
        }
        TabButton {
            text: qsTr("Phonebook & FRITZ!Box")
            icon.name: "address-book-new"
        }
        TabButton {
            text: qsTr("Audio & Devices")
            icon.name: "audio-card"
        }
    }

    StackLayout {
        anchors.fill: parent
        currentIndex: settingsTabBar.currentIndex

        // ---------------------------------------------------------------------
        // Tab 0: SIP Accounts & PBX Configuration
        // ---------------------------------------------------------------------
        AccountWizardView {
            id: accountsView
            bridge: page.bridge
        }

        // ---------------------------------------------------------------------
        // Tab 1: FRITZ!Box Phonebook Sync & Call Behavior
        // ---------------------------------------------------------------------
        Kirigami.ScrollablePage {
            id: phonebookSettingsPage
            title: qsTr("Phonebook & PBX")

            ColumnLayout {
                width: parent.width
                spacing: Kirigami.Units.largeSpacing

                // 1. FRITZ!Box Phonebook Card
                Kirigami.Card {
                    Layout.fillWidth: true

                    header: Kirigami.Heading {
                        text: qsTr("AVM FRITZ!Box Phonebook Synchronization")
                        level: 3
                    }

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.mediumSpacing

                        Label {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: qsTr("Plasma SIP synchronizes your FRITZ!Box phonebook via TR-064 (HTTP port 49000) using your configured SIP account credentials. Contacts, internal numbers, and quickdials are stored securely in local encrypted SQLite.")
                            color: Kirigami.Theme.disabledTextColor
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }

                        // Status Row
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.smallSpacing

                            Rectangle {
                                implicitWidth: 10
                                implicitHeight: 10
                                radius: 5
                                color: page.bridge && page.bridge.is_syncing_contacts ?
                                       Kirigami.Theme.highlightColor :
                                       (page.contactsCount > 0 ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.neutralTextColor)
                            }

                            Label {
                                text: page.bridge && page.bridge.is_syncing_contacts ?
                                      qsTr("Synchronizing with FRITZ!Box...") :
                                      (page.contactsCount > 0 ?
                                       qsTr("%1 contacts synchronized in local database").arg(page.contactsCount) :
                                       qsTr("No phonebook contacts synchronized yet"))
                                font.weight: Font.Medium
                                color: Kirigami.Theme.textColor
                            }
                        }

                        // Last Sync Status Message (if any)
                        Label {
                            Layout.fillWidth: true
                            visible: page.bridge && page.bridge.contact_sync_status !== ""
                            text: page.bridge ? page.bridge.contact_sync_status : ""
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                            color: page.bridge && (page.bridge.contact_sync_status.indexOf("failed") !== -1 || page.bridge.contact_sync_status.indexOf("Error") !== -1) ?
                                   Kirigami.Theme.negativeTextColor : Kirigami.Theme.positiveTextColor
                            wrapMode: Text.WordWrap
                        }

                        // Action Button
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.smallSpacing

                            Button {
                                id: syncBtn
                                text: qsTr("Synchronize Phonebook Now")
                                icon.name: "view-refresh"
                                enabled: page.bridge && !page.bridge.is_syncing_contacts
                                onClicked: {
                                    if (page.bridge) page.bridge.sync_phonebook();
                                }
                            }

                            BusyIndicator {
                                running: page.bridge && page.bridge.is_syncing_contacts
                                visible: running
                                Layout.preferredWidth: 24
                                Layout.preferredHeight: 24
                            }
                        }
                    }
                }

                // 2. Call Decline / Reject Behavior Card
                Kirigami.Card {
                    Layout.fillWidth: true

                    header: Kirigami.Heading {
                        text: qsTr("Call Rejection Behavior")
                        level: 3
                    }

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        Label {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: qsTr("Select the SIP response code sent to callers when you reject an incoming call:")
                            color: Kirigami.Theme.disabledTextColor
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }

                        ComboBox {
                            id: rejectCombo
                            Layout.fillWidth: true
                            model: [
                                { label: qsTr("603 Decline (Standard decline response)"), value: "603_decline" },
                                { label: qsTr("486 Busy Here (Indicates line is busy)"), value: "486_busy" },
                                { label: qsTr("480 Temporarily Unavailable (Reroutes to voicemail if configured)"), value: "480_unavailable" }
                            ]
                            textRole: "label"

                            currentIndex: {
                                var cur = page.bridge ? page.bridge.reject_mode : "603_decline";
                                for (var i = 0; i < model.length; i++) {
                                    if (model[i].value === cur) return i;
                                }
                                return 0;
                            }

                            onActivated: function(index) {
                                var selected = model[index].value;
                                if (page.bridge) {
                                    page.bridge.change_reject_mode(selected);
                                }
                            }
                        }
                    }
                }

                // 3. Caller Resolution & Privacy Card
                Kirigami.Card {
                    Layout.fillWidth: true

                    header: Kirigami.Heading {
                        text: qsTr("Caller Name Resolution & Privacy")
                        level: 3
                    }

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        RowLayout {
                            spacing: Kirigami.Units.smallSpacing
                            Kirigami.Icon {
                                source: "security-high"
                                implicitWidth: 16
                                implicitHeight: 16
                                color: Kirigami.Theme.positiveTextColor
                            }
                            Label {
                                text: qsTr("100% Offline & Private")
                                font.weight: Font.DemiBold
                            }
                        }

                        Label {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: qsTr("Phonebook synchronization and caller name lookups occur strictly between Plasma SIP and your local PBX/FRITZ!Box. No contact data or call logs are transmitted to external servers or cloud services.")
                            color: Kirigami.Theme.disabledTextColor
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }
                    }
                }
            }
        }

        // ---------------------------------------------------------------------
        // Tab 2: Audio Devices, Ringtone & Sound
        // ---------------------------------------------------------------------
        AudioSettingsView {
            id: audioView
            bridge: page.bridge
        }
    }
}
