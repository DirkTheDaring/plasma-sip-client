import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Call History")

    property var bridge: null
    signal redialRequested(string target)

    property int filterIndex: 0 // 0 = All, 1 = Missed

    property var rawHistoryList: {
        if (!page.bridge) return [];
        try {
            var raw = page.bridge.call_history_json;
            if (!raw || raw === "") return [];
            return JSON.parse(raw);
        } catch (e) {
            return [];
        }
    }

    // Helper: Formats German mobile and landline numbers for visual clarity
    function formatPhoneNumber(num) {
        if (!num) return "";
        var s = ("" + num).trim();
        // German mobile (015x, 016x, 017x)
        if (/^01[567]\d{7,9}$/.test(s)) {
            return s.substring(0, 4) + " " + s.substring(4);
        }
        // International formatted e.g. +49176...
        if (/^\+491[567]\d{7,9}$/.test(s)) {
            return s.substring(0, 3) + " " + s.substring(3, 7) + " " + s.substring(7);
        }
        // German standard landline (02xx, 03xx, 07xx, etc.)
        if (/^0[2-9]\d{9,11}$/.test(s)) {
            return s.substring(0, 4) + " " + s.substring(4);
        }
        return s;
    }

    // Helper: Extract uppercase initials from a contact name (e.g. "Alice Smith" -> "AS")
    function getInitials(name) {
        if (!name) return "";
        var parts = name.trim().split(/\s+/);
        if (parts.length >= 2 && parts[0].length > 0 && parts[1].length > 0) {
            return (parts[0][0] + parts[1][0]).toUpperCase();
        } else if (parts.length === 1 && parts[0].length > 0) {
            return parts[0][0].toUpperCase();
        }
        return "";
    }

    // Helper: Format duration into readable "Xm Ys" or "Xs"
    function formatDuration(secs) {
        if (!secs || secs <= 0) return "";
        var m = Math.floor(secs / 60);
        var s = secs % 60;
        if (m > 0) {
            return m + "m " + (s < 10 ? "0" : "") + s + "s";
        }
        return s + "s";
    }

    // Helper: Section header date ("Today", "Yesterday", or localized date)
    function getSectionDate(isoString) {
        if (!isoString) return qsTr("Unknown Date");
        var d = new Date(isoString);
        if (isNaN(d.getTime())) return qsTr("Unknown Date");

        var now = new Date();
        var today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
        var callDay = new Date(d.getFullYear(), d.getMonth(), d.getDate());

        var diffMs = today.getTime() - callDay.getTime();
        var diffDays = Math.round(diffMs / (1000 * 60 * 60 * 24));

        if (diffDays === 0) {
            return qsTr("Today");
        } else if (diffDays === 1) {
            return qsTr("Yesterday");
        } else if (diffDays > 1 && diffDays < 7) {
            return d.toLocaleDateString(Qt.locale(), "dddd");
        } else {
            return d.toLocaleDateString(Qt.locale(), Locale.LongFormat);
        }
    }

    // Helper: Time of day (e.g. "14:23")
    function formatTimeOfDay(isoString) {
        if (!isoString) return "";
        var d = new Date(isoString);
        if (isNaN(d.getTime())) return "";
        return d.toLocaleTimeString(Qt.locale(), Locale.ShortFormat);
    }

    // Process raw records into clean structured model items
    property var processedList: {
        var raw = page.rawHistoryList;
        var result = [];
        for (var i = 0; i < raw.length; i++) {
            var item = raw[i];
            var isoTime = item.started_at || item.start_time || "";
            var duration = item.duration_seconds || item.duration_secs || 0;
            var direction = (item.direction || "").toLowerCase();
            var reason = (item.disconnect_reason || "").toLowerCase();

            var displayName = (item.display_name || "").trim();
            var phoneNumber = (item.phone_number || "").trim();
            var remote = (item.remote_party || "").trim();

            // Fallback parsing if display_name and phone_number were not separated
            if (displayName === "" && phoneNumber === "") {
                var parenStart = remote.indexOf("(");
                var parenEnd = remote.lastIndexOf(")");
                if (parenStart > 0 && parenEnd > parenStart) {
                    displayName = remote.substring(0, parenStart).trim();
                    phoneNumber = remote.substring(parenStart + 1, parenEnd).trim();
                } else {
                    phoneNumber = remote;
                }
            }

            var isOutbound = (direction === "outbound");
            var isMissed = false;
            var statusLabel = "";
            var statusIcon = "";
            var statusColor = Kirigami.Theme.textColor;

            if (isOutbound) {
                if (duration > 0) {
                    statusLabel = qsTr("Outgoing");
                    statusIcon = "arrow-up-right";
                    statusColor = Kirigami.Theme.highlightColor;
                } else {
                    statusLabel = reason.includes("cancel") ? qsTr("Cancelled") : qsTr("Unanswered");
                    statusIcon = "arrow-up-right";
                    statusColor = Kirigami.Theme.disabledTextColor;
                    isMissed = true;
                }
            } else {
                // Inbound
                if (duration > 0) {
                    statusLabel = qsTr("Incoming");
                    statusIcon = "call-start";
                    statusColor = Kirigami.Theme.positiveTextColor;
                } else {
                    isMissed = true;
                    statusIcon = "call-stop";
                    statusColor = Kirigami.Theme.negativeTextColor;
                    if (reason.includes("reject") || reason.includes("decline")) {
                        statusLabel = qsTr("Declined");
                    } else if (reason.includes("busy")) {
                        statusLabel = qsTr("Busy");
                    } else {
                        statusLabel = qsTr("Missed call");
                    }
                }
            }

            var durStr = formatDuration(duration);
            var subtitle = "";
            if (displayName !== "" && phoneNumber !== "") {
                subtitle = formatPhoneNumber(phoneNumber) + " • " + statusLabel + (durStr !== "" ? (" • " + durStr) : "");
            } else {
                subtitle = statusLabel + (durStr !== "" ? (" • " + durStr) : "");
            }

            var title = displayName !== "" ? displayName : (formatPhoneNumber(phoneNumber) || remote || qsTr("Unknown"));

            result.push({
                id: item.id || "",
                account_id: item.account_id || "",
                direction: direction,
                remote_party: remote,
                displayName: displayName,
                phoneNumber: phoneNumber,
                displayTitle: title,
                subtitleText: subtitle,
                duration: duration,
                durationStr: durStr,
                isoTime: isoTime,
                timeOfDay: formatTimeOfDay(isoTime),
                sectionDate: getSectionDate(isoTime),
                disconnect_reason: item.disconnect_reason || "",
                isMissed: isMissed,
                statusLabel: statusLabel,
                statusIcon: statusIcon,
                statusColor: statusColor,
                avatarInitials: getInitials(displayName)
            });
        }
        return result;
    }

    // Filtered list based on active tab
    property var filteredList: {
        var all = page.processedList;
        if (page.filterIndex === 1) {
            return all.filter(function(entry) { return entry.isMissed; });
        }
        return all;
    }

    // Selected entry for details sheet
    property var selectedEntry: null

    actions: [
        Kirigami.Action {
            text: qsTr("Clear All")
            icon.name: "edit-clear-history"
            enabled: page.rawHistoryList.length > 0
            onTriggered: clearConfirmDialog.open()
        },
        Kirigami.Action {
            text: qsTr("Refresh")
            icon.name: "view-refresh"
            checkable: false
            onTriggered: {
                if (page.bridge) page.bridge.refresh_history();
            }
        }
    ]

    header: ColumnLayout {
        spacing: 0
        TabBar {
            id: historyFilterBar
            Layout.fillWidth: true
            currentIndex: page.filterIndex
            onCurrentIndexChanged: page.filterIndex = currentIndex

            TabButton {
                text: qsTr("All (%1)").arg(page.rawHistoryList.length)
            }
            TabButton {
                text: {
                    var missedCount = 0;
                    for (var i = 0; i < page.processedList.length; i++) {
                        if (page.processedList[i].isMissed) missedCount++;
                    }
                    return qsTr("Missed (%1)").arg(missedCount);
                }
            }
        }
    }

    Kirigami.PlaceholderMessage {
        anchors.centerIn: parent
        width: parent.width - (Kirigami.Units.gridUnit * 4)
        visible: page.filteredList.length === 0
        text: page.filterIndex === 1 ? qsTr("No Missed Calls") : qsTr("No Call History")
        explanation: page.filterIndex === 1 ?
            qsTr("You have no missed or unanswered calls.") :
            qsTr("Calls you place or receive will appear here.")
        icon.name: page.filterIndex === 1 ? "call-stop" : "view-history"
    }

    ListView {
        id: historyListView
        anchors.fill: parent
        visible: page.filteredList.length > 0
        model: page.filteredList
        clip: true

        section.property: "sectionDate"
        section.delegate: Kirigami.ListSectionHeader {
            text: section
            width: ListView.view ? ListView.view.width : parent.width
        }

        delegate: ItemDelegate {
            id: callDelegate
            width: ListView.view ? ListView.view.width : parent.width
            hoverEnabled: true

            onClicked: {
                page.selectedEntry = modelData;
                callDetailSheet.open();
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                // Contact Avatar / Initial Circle
                Rectangle {
                    implicitWidth: 38
                    implicitHeight: 38
                    radius: width / 2
                    color: modelData.isMissed ?
                           Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.14) :
                           Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.12)

                    border.color: modelData.isMissed ?
                                  Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.3) :
                                  Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.25)
                    border.width: 1

                    Label {
                        anchors.centerIn: parent
                        visible: modelData.avatarInitials !== ""
                        text: modelData.avatarInitials
                        font.bold: true
                        font.pointSize: 12
                        color: modelData.isMissed ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.highlightColor
                    }

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        visible: modelData.avatarInitials === ""
                        source: modelData.isMissed ? "call-stop" : (modelData.direction === "outbound" ? "arrow-up-right" : "call-start")
                        implicitWidth: 18
                        implicitHeight: 18
                        color: modelData.statusColor
                    }
                }

                // Middle: Caller Title & Status Subtitle
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Label {
                        Layout.fillWidth: true
                        text: modelData.displayTitle
                        font.bold: true
                        font.pointSize: 11
                        elide: Text.ElideRight
                        color: modelData.isMissed ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 4

                        Kirigami.Icon {
                            source: modelData.statusIcon
                            implicitWidth: 12
                            implicitHeight: 12
                            color: modelData.statusColor
                        }

                        Label {
                            Layout.fillWidth: true
                            text: modelData.subtitleText
                            font.pointSize: 9
                            elide: Text.ElideRight
                            color: modelData.isMissed ?
                                   Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.85) :
                                   Kirigami.Theme.disabledTextColor
                        }
                    }
                }

                // Trailing: Time of Day & Redial Action Button
                RowLayout {
                    spacing: Kirigami.Units.smallSpacing

                    Label {
                        text: modelData.timeOfDay
                        font.pointSize: 9
                        color: Kirigami.Theme.disabledTextColor
                        horizontalAlignment: Text.AlignRight
                    }

                    ToolButton {
                        icon.name: "call-start"
                        ToolTip.text: qsTr("Call %1").arg(modelData.phoneNumber || modelData.displayTitle)
                        ToolTip.visible: hovered
                        Accessible.name: ToolTip.text
                        Accessible.role: Accessible.Button
                        onClicked: {
                            var target = modelData.phoneNumber || modelData.remote_party;
                            if (target) {
                                page.redialRequested(target);
                            }
                        }
                    }
                }
            }
        }
    }

    // Call Details Overlay Sheet
    Kirigami.OverlaySheet {
        id: callDetailSheet
        title: qsTr("Call Details")

        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            implicitWidth: Math.min(page.width - (Kirigami.Units.gridUnit * 2), Kirigami.Units.gridUnit * 22)

            // Contact Header
            RowLayout {
                spacing: Kirigami.Units.mediumSpacing
                Layout.fillWidth: true

                Rectangle {
                    implicitWidth: 48
                    implicitHeight: 48
                    radius: width / 2
                    color: (page.selectedEntry && page.selectedEntry.isMissed) ?
                           Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.15) :
                           Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)

                    Label {
                        anchors.centerIn: parent
                        visible: page.selectedEntry && page.selectedEntry.avatarInitials !== ""
                        text: page.selectedEntry ? page.selectedEntry.avatarInitials : ""
                        font.bold: true
                        font.pointSize: 15
                        color: (page.selectedEntry && page.selectedEntry.isMissed) ?
                               Kirigami.Theme.negativeTextColor : Kirigami.Theme.highlightColor
                    }

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        visible: !page.selectedEntry || page.selectedEntry.avatarInitials === ""
                        source: (page.selectedEntry && page.selectedEntry.isMissed) ? "call-stop" : "call-start"
                        implicitWidth: 24
                        implicitHeight: 24
                        color: page.selectedEntry ? page.selectedEntry.statusColor : Kirigami.Theme.textColor
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Label {
                        text: page.selectedEntry ? page.selectedEntry.displayTitle : ""
                        font.bold: true
                        font.pointSize: 14
                        elide: Text.ElideRight
                    }
                    Label {
                        visible: page.selectedEntry && page.selectedEntry.displayName !== "" && page.selectedEntry.phoneNumber !== ""
                        text: page.selectedEntry ? page.formatPhoneNumber(page.selectedEntry.phoneNumber) : ""
                        font.pointSize: 11
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
            }

            Kirigami.Separator {
                Layout.fillWidth: true
            }

            // Call Metadata
            GridLayout {
                columns: 2
                columnSpacing: Kirigami.Units.largeSpacing
                rowSpacing: Kirigami.Units.smallSpacing
                Layout.fillWidth: true

                Label {
                    text: qsTr("Type:")
                    color: Kirigami.Theme.disabledTextColor
                }
                Label {
                    text: page.selectedEntry ? (page.selectedEntry.statusLabel + (page.selectedEntry.direction === "outbound" ? qsTr(" (Outgoing)") : qsTr(" (Incoming)"))) : ""
                    font.bold: true
                    color: page.selectedEntry ? page.selectedEntry.statusColor : Kirigami.Theme.textColor
                }

                Label {
                    text: qsTr("Date & Time:")
                    color: Kirigami.Theme.disabledTextColor
                }
                Label {
                    text: {
                        if (!page.selectedEntry || !page.selectedEntry.isoTime) return "";
                        var d = new Date(page.selectedEntry.isoTime);
                        if (isNaN(d.getTime())) return page.selectedEntry.isoTime;
                        return d.toLocaleString(Qt.locale(), Locale.LongFormat);
                    }
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Label {
                    text: qsTr("Duration:")
                    color: Kirigami.Theme.disabledTextColor
                }
                Label {
                    text: (page.selectedEntry && page.selectedEntry.duration > 0) ?
                          page.selectedEntry.durationStr : qsTr("0s (Not connected)")
                }

                Label {
                    visible: page.selectedEntry && page.selectedEntry.disconnect_reason !== ""
                    text: qsTr("Reason:")
                    color: Kirigami.Theme.disabledTextColor
                }
                Label {
                    visible: page.selectedEntry && page.selectedEntry.disconnect_reason !== ""
                    text: page.selectedEntry ? page.selectedEntry.disconnect_reason : ""
                    color: Kirigami.Theme.disabledTextColor
                }
            }

            Kirigami.Separator {
                Layout.fillWidth: true
            }

            // Action Buttons
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Button {
                    text: qsTr("Call")
                    icon.name: "call-start"
                    highlighted: true
                    Layout.fillWidth: true
                    onClicked: {
                        var target = page.selectedEntry ? (page.selectedEntry.phoneNumber || page.selectedEntry.remote_party) : "";
                        callDetailSheet.close();
                        if (target) {
                            page.redialRequested(target);
                        }
                    }
                }

                Button {
                    text: qsTr("Copy Number")
                    icon.name: "edit-copy"
                    Layout.fillWidth: true
                    onClicked: {
                        var num = page.selectedEntry ? (page.selectedEntry.phoneNumber || page.selectedEntry.remote_party) : "";
                        if (num) {
                            clipHelper.text = num;
                            clipHelper.selectAll();
                            clipHelper.copy();
                        }
                        callDetailSheet.close();
                    }
                }

                Button {
                    text: qsTr("Delete")
                    icon.name: "edit-delete"
                    Layout.fillWidth: true
                    onClicked: {
                        var id = page.selectedEntry ? page.selectedEntry.id : "";
                        callDetailSheet.close();
                        if (id && page.bridge) {
                            page.bridge.delete_history_item(id);
                        }
                    }
                }
            }
        }
    }

    // Hidden helper for copying text to system clipboard
    TextInput {
        id: clipHelper
        visible: false
    }

    // Confirmation Dialog for Clear All History
    Dialog {
        id: clearConfirmDialog
        title: qsTr("Clear Call History")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Cancel | Dialog.Ok

        Label {
            text: qsTr("Are you sure you want to delete all call history records?\nThis action cannot be undone.")
            wrapMode: Text.WordWrap
        }

        onAccepted: {
            if (page.bridge) {
                page.bridge.clear_history();
            }
        }
    }
}
