import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Dial Pad")
    focus: true

    property var bridge: null
    property bool isSilent: {
        if (!page.bridge || !page.bridge.audio_config_json) return false;
        try {
            var parsed = JSON.parse(page.bridge.audio_config_json);
            return parsed && parsed.silent_mode === true;
        } catch (e) {
            return false;
        }
    }

    // Helper to format account summary nicely without leaking raw URIs
    readonly property string displayAccountName: {
        if (!page.bridge) return qsTr("Connecting to daemon...");
        var raw = page.bridge.account_summary || "";
        if (!page.bridge.is_registered) {
            return raw.length > 0 ? raw : qsTr("Not Registered");
        }
        // Extract friendly label if in format "Registered: Name (user@host)"
        var match = raw.match(/Registered:\s*([^(]+)/);
        if (match && match[1]) {
            return match[1].trim();
        }
        return raw;
    }

    readonly property var allContacts: {
        if (!page.bridge || !page.bridge.contacts_json) return [];
        try {
            var raw = page.bridge.contacts_json;
            if (!raw || raw === "" || raw === "[]") return [];
            return JSON.parse(raw);
        } catch (e) {
            return [];
        }
    }

    readonly property var matchedContacts: {
        var query = dialInput.text.trim();
        if (query.length === 0 || allContacts.length === 0) return [];
        var cleanQ = query.replace(/[^\d+*#]/g, "").toLowerCase();
        var results = [];
        for (var i = 0; i < allContacts.length && results.length < 3; i++) {
            var c = allContacts[i];
            var matchedNum = null;
            if (c.numbers) {
                for (var j = 0; j < c.numbers.length; j++) {
                    var n = c.numbers[j].number.replace(/[^\d+*#]/g, "");
                    if (n.indexOf(cleanQ) !== -1 || (cleanQ.length >= 2 && n.endsWith(cleanQ))) {
                        matchedNum = c.numbers[j].number;
                        break;
                    }
                }
            }
            if (matchedNum || (c.quickdial && ("**" + c.quickdial === query || c.quickdial === query))) {
                results.push({
                    name: c.name,
                    number: matchedNum || (c.numbers && c.numbers.length > 0 ? c.numbers[0].number : query),
                    quickdial: c.quickdial
                });
            } else if (c.name && c.name.toLowerCase().indexOf(query.toLowerCase()) !== -1) {
                results.push({
                    name: c.name,
                    number: c.numbers && c.numbers.length > 0 ? c.numbers[0].number : query,
                    quickdial: c.quickdial
                });
            }
        }
        return results;
    }

    readonly property var audioConfig: {
        if (!page.bridge || !page.bridge.audio_config_json) return {};
        try {
            return JSON.parse(page.bridge.audio_config_json) || {};
        } catch (e) {
            return {};
        }
    }

    readonly property var audioDevices: {
        if (!page.bridge || !page.bridge.audio_devices_json) return { inputs: [], outputs: [] };
        try {
            return JSON.parse(page.bridge.audio_devices_json) || { inputs: [], outputs: [] };
        } catch (e) {
            return { inputs: [], outputs: [] };
        }
    }

    readonly property string currentOutputDeviceName: {
        var devId = audioConfig.output_device_id;
        var outs = audioDevices.outputs || [];
        if (!devId || devId === "default") {
            return outs.length > 0 ? outs[0].name : qsTr("Default Audio");
        }
        for (var i = 0; i < outs.length; i++) {
            if (outs[i].id === devId) return outs[i].name;
        }
        return outs.length > 0 ? outs[0].name : qsTr("Default Audio");
    }

    readonly property string currentInputDeviceName: {
        var devId = audioConfig.input_device_id;
        var ins = audioDevices.inputs || [];
        if (!devId || devId === "default") {
            return ins.length > 0 ? ins[0].name : qsTr("Default Microphone");
        }
        for (var j = 0; j < ins.length; j++) {
            if (ins[j].id === devId) return ins[j].name;
        }
        return ins.length > 0 ? ins[0].name : qsTr("Default Microphone");
    }

    signal startCallRequested(string target)
    signal openAccountsRequested()

    actions: [
        Kirigami.Action {
            text: page.isSilent ? qsTr("Silent Mode: ON") : qsTr("Silent Mode: OFF")
            icon.name: page.isSilent ? "audio-volume-muted" : "audio-volume-high"
            tooltip: qsTr("Toggle Silent Mode (Mute Ringtone)")
            checkable: true
            checked: page.isSilent
            onTriggered: {
                if (page.bridge) {
                    page.bridge.set_silent_mode(!page.isSilent);
                }
            }
        },
        Kirigami.Action {
            text: qsTr("Audio Devices")
            icon.name: "audio-headset"
            tooltip: qsTr("Quick Audio Device & Volume Settings")
            onTriggered: {
                if (page.bridge) page.bridge.refresh_devices();
                audioQuickSheet.open();
            }
        }
    ]

    function prefillNumber(number) {
        dialInput.text = number;
        dialInput.cursorPosition = dialInput.text.length;
        dialInput.forceActiveFocus();
    }

    Component.onCompleted: dialInput.forceActiveFocus()
    onVisibleChanged: {
        if (visible) dialInput.forceActiveFocus();
    }

    // Keyboard support: Enter calls, Backspace deletes, Esc clears, numbers append
    Keys.onPressed: function(event) {
        if (event.key === Qt.Key_Enter || event.key === Qt.Key_Return) {
            if (callButton.canCall) {
                page.startCallRequested(dialInput.text.trim());
                event.accepted = true;
                return;
            }
        }
        if (event.key === Qt.Key_Backspace) {
            if (dialInput.text.length > 0 && !dialInput.activeFocus) {
                dialInput.text = dialInput.text.slice(0, -1);
                dialInput.forceActiveFocus();
                event.accepted = true;
                return;
            }
        }
        if (event.key === Qt.Key_Escape) {
            dialInput.text = "";
            dialInput.forceActiveFocus();
            event.accepted = true;
            return;
        }

        // Direct keypad and keyboard numbers/symbols to dialInput if it somehow lost focus
        if (!dialInput.activeFocus && event.text.length > 0) {
            var validChars = "0123456789*#+";
            if (validChars.indexOf(event.text) !== -1) {
                dialInput.text += event.text;
                dialInput.forceActiveFocus();
                event.accepted = true;
                return;
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        // ---------------------------------------------------------------------
        // 1. Dual Status Badges: Account Status Pill & Quick Audio Selector Pill
        // ---------------------------------------------------------------------
        RowLayout {
            id: headerPillsRow
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.mediumSpacing

            // Account status badge
            Rectangle {
                id: statusPill
                implicitHeight: 28
                implicitWidth: statusRow.implicitWidth + 20
                radius: 14
                color: page.bridge && page.bridge.is_registered ?
                       Qt.rgba(Kirigami.Theme.positiveTextColor.r, Kirigami.Theme.positiveTextColor.g, Kirigami.Theme.positiveTextColor.b, 0.12) :
                       (page.bridge && page.bridge.account_summary.indexOf("Failed") !== -1 ?
                        Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.12) :
                        Qt.rgba(Kirigami.Theme.neutralTextColor.r, Kirigami.Theme.neutralTextColor.g, Kirigami.Theme.neutralTextColor.b, 0.10))
                border.color: page.bridge && page.bridge.is_registered ?
                              Qt.rgba(Kirigami.Theme.positiveTextColor.r, Kirigami.Theme.positiveTextColor.g, Kirigami.Theme.positiveTextColor.b, 0.35) :
                              Qt.rgba(Kirigami.Theme.disabledTextColor.r, Kirigami.Theme.disabledTextColor.g, Kirigami.Theme.disabledTextColor.b, 0.25)
                border.width: 1

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: page.openAccountsRequested()
                    ToolTip.text: qsTr("Account Settings & Details")
                    ToolTip.visible: containsMouse
                }

                RowLayout {
                    id: statusRow
                    anchors.centerIn: parent
                    spacing: 6

                    Rectangle {
                        Layout.preferredWidth: 8
                        Layout.preferredHeight: 8
                        radius: 4
                        color: page.bridge && page.bridge.is_registered ? Kirigami.Theme.positiveTextColor :
                               (page.bridge && page.bridge.account_summary.indexOf("Failed") !== -1 ? Kirigami.Theme.negativeTextColor :
                                (page.bridge && page.bridge.daemon_connected ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.negativeTextColor))
                    }

                    Label {
                        text: page.displayAccountName
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        font.weight: Font.Medium
                        color: page.bridge && page.bridge.is_registered ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
                    }

                    Label {
                        visible: page.isSilent
                        text: "• " + qsTr("Silent")
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        font.bold: true
                        color: Kirigami.Theme.neutralTextColor
                    }
                }
            }

            // Quick Audio Device Pill
            Rectangle {
                id: audioPill
                implicitHeight: 28
                implicitWidth: audioPillRow.implicitWidth + 20
                radius: 14
                color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.10)
                border.color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.30)
                border.width: 1

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        if (page.bridge) page.bridge.refresh_devices();
                        audioQuickSheet.open();
                    }
                    ToolTip.text: qsTr("Audio: %1 (Click to change)").arg(page.currentOutputDeviceName)
                    ToolTip.visible: containsMouse
                }

                RowLayout {
                    id: audioPillRow
                    anchors.centerIn: parent
                    spacing: 6

                    Kirigami.Icon {
                        source: "audio-headset"
                        implicitWidth: 14
                        implicitHeight: 14
                        color: Kirigami.Theme.highlightColor
                    }

                    Label {
                        text: {
                            var n = page.currentOutputDeviceName;
                            return n.length > 22 ? n.substring(0, 20) + "…" : n;
                        }
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        font.weight: Font.Medium
                        color: Kirigami.Theme.textColor
                    }

                    Kirigami.Icon {
                        source: "arrow-down"
                        implicitWidth: 10
                        implicitHeight: 10
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
            }
        }

        // Inline Warning Banner when no SIP account is registered
        Kirigami.InlineMessage {
            id: noAccountWarning
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: page.bridge ? !page.bridge.is_registered : false
            text: page.bridge && (page.bridge.accounts_json === "[]" && page.bridge.account_summary === "No accounts configured") ?
                  qsTr("No SIP accounts configured. Please add your FRITZ!Box or SIP account to make calls.") :
                  qsTr("SIP account is not registered. Check credentials or connection to your PBX/FRITZ!Box.")
            actions: [
                Kirigami.Action {
                    text: qsTr("Reconnect")
                    icon.name: "network-connect"
                    visible: page.bridge ? page.bridge.accounts_json !== "[]" && page.bridge.account_summary !== "No accounts configured" : false
                    onTriggered: {
                        if (page.bridge) page.bridge.reconnect();
                    }
                },
                Kirigami.Action {
                    text: qsTr("Accounts & PBX")
                    icon.name: "network-server"
                    checkable: false
                    onTriggered: page.openAccountsRequested()
                }
            ]
        }

        Item { Layout.preferredHeight: 4 }

        // ---------------------------------------------------------------------
        // 2. Hero Phone Number Display (Clean, borderless typography)
        // ---------------------------------------------------------------------
        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignHCenter
            spacing: 2

            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: 52

                TextInput {
                    id: dialInput
                    anchors.centerIn: parent
                    width: Math.min(parent.width - 64, implicitWidth + 20)
                    horizontalAlignment: TextInput.AlignHCenter
                    verticalAlignment: TextInput.AlignVCenter
                    font.pointSize: text.length > 16 ? 20 : (text.length > 12 ? 24 : 30)
                    font.weight: Font.DemiBold
                    font.family: "Noto Sans, Inter, Roboto, sans-serif"
                    color: Kirigami.Theme.textColor
                    selectionColor: Kirigami.Theme.highlightColor
                    selectedTextColor: Kirigami.Theme.highlightedTextColor
                    selectByMouse: true
                    focus: true

                    // Subtle blinking cursor
                    cursorVisible: activeFocus

                    onAccepted: {
                        if (callButton.canCall) {
                            page.startCallRequested(dialInput.text.trim());
                        }
                    }

                    // Placeholder text when empty
                    Text {
                        anchors.centerIn: parent
                        text: qsTr("Enter number or extension...")
                        color: Kirigami.Theme.disabledTextColor
                        font.pointSize: 18
                        font.weight: Font.Normal
                        visible: dialInput.text.length === 0 && !dialInput.activeFocus
                    }
                }

                // Inline soft clear button next to the typed number
                RoundButton {
                    id: inlineClearBtn
                    anchors.right: parent.right
                    anchors.rightMargin: Kirigami.Units.largeSpacing
                    anchors.verticalCenter: parent.verticalCenter
                    implicitWidth: 32
                    implicitHeight: 32
                    visible: dialInput.text.length > 0
                    focusPolicy: Qt.NoFocus
                    icon.name: "edit-clear"
                    icon.width: 16
                    icon.height: 16
                    opacity: dialInput.text.length > 0 ? 0.75 : 0.0
                    Behavior on opacity { NumberAnimation { duration: 120 } }
                    background: Rectangle {
                        radius: 16
                        color: inlineClearBtn.hovered ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12) : "transparent"
                    }
                    onClicked: {
                        dialInput.text = "";
                        dialInput.forceActiveFocus();
                    }
                    ToolTip.text: qsTr("Clear (Esc)")
                    ToolTip.visible: hovered
                }
            }

            // Real-time Context / Extension Label
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: {
                    var t = dialInput.text.trim();
                    if (t.length === 0) return qsTr("Ready to dial");
                    if (t === "**610") return qsTr("FRITZ!Box Handset 1");
                    if (t === "**611") return qsTr("FRITZ!Box Handset 2");
                    if (t === "**612") return qsTr("FRITZ!Box Handset 3");
                    if (t === "**9") return qsTr("Broadcast to All Connected Phones");
                    if (t.startsWith("**")) return qsTr("Internal PBX Extension");
                    if (t.startsWith("+49") || t.startsWith("0049")) return qsTr("Germany");
                    if (t.startsWith("+1") || t.startsWith("001")) return qsTr("North America");
                    if (t.startsWith("+44") || t.startsWith("0044")) return qsTr("United Kingdom");
                    if (t.startsWith("+43") || t.startsWith("0043")) return qsTr("Austria");
                    if (t.startsWith("+41") || t.startsWith("0041")) return qsTr("Switzerland");
                    if (t.startsWith("+")) return qsTr("International Number");
                    return qsTr("Direct Call");
                }
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                font.weight: Font.Medium
                color: dialInput.text.trim().length > 0 ? Kirigami.Theme.highlightColor : Kirigami.Theme.disabledTextColor
            }
        }

        Item { Layout.preferredHeight: 4 }

        // ---------------------------------------------------------------------
        // 3. Quick Extension & T9 Contact Matches Strip
        // ---------------------------------------------------------------------
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.smallSpacing

            // Case A: Show matched contacts when dialInput has text and matches found
            Repeater {
                model: page.matchedContacts

                Rectangle {
                    implicitHeight: 28
                    implicitWidth: matchChipRow.implicitWidth + 18
                    radius: 14
                    color: matchMouse.pressed ?
                           Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.25) :
                           (matchMouse.containsMouse ?
                            Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15) :
                            Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.08))
                    border.color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.3)
                    border.width: 1

                    scale: matchMouse.pressed ? 0.95 : 1.0
                    Behavior on scale { NumberAnimation { duration: 60 } }

                    MouseArea {
                        id: matchMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            dialInput.text = modelData.number;
                            dialInput.cursorPosition = dialInput.text.length;
                            dialInput.forceActiveFocus();
                        }
                        ToolTip.text: qsTr("Call %1 (%2)").arg(modelData.name).arg(modelData.number)
                        ToolTip.visible: containsMouse
                    }

                    RowLayout {
                        id: matchChipRow
                        anchors.centerIn: parent
                        spacing: 5

                        Kirigami.Icon {
                            source: "user-identity"
                            implicitWidth: 12
                            implicitHeight: 12
                            color: Kirigami.Theme.highlightColor
                        }

                        Label {
                            text: modelData.name
                            font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.95
                            font.weight: Font.DemiBold
                            color: Kirigami.Theme.textColor
                        }

                        Label {
                            text: modelData.number
                            font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.85
                            color: Kirigami.Theme.disabledTextColor
                        }
                    }
                }
            }

            // Case B: Show PBX Quick Extensions when dialInput is empty
            Repeater {
                model: page.matchedContacts.length === 0 ? [
                    { label: qsTr("Handset 1"), ext: "**610", icon: "phone" },
                    { label: qsTr("Handset 2"), ext: "**611", icon: "phone" },
                    { label: qsTr("All Phones"), ext: "**9", icon: "audio-volume-high" }
                ] : []

                Rectangle {
                    implicitHeight: 28
                    implicitWidth: chipRow.implicitWidth + 20
                    radius: 14
                    color: chipMouse.pressed ?
                           Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.20) :
                           (chipMouse.containsMouse ?
                            Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10) :
                            Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05))
                    border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12)
                    border.width: 1

                    scale: chipMouse.pressed ? 0.95 : 1.0
                    Behavior on scale { NumberAnimation { duration: 60 } }

                    MouseArea {
                        id: chipMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            dialInput.text = modelData.ext;
                            dialInput.forceActiveFocus();
                        }
                        ToolTip.text: qsTr("Dial extension %1 (%2)").arg(modelData.ext).arg(modelData.label)
                        ToolTip.visible: containsMouse
                    }

                    RowLayout {
                        id: chipRow
                        anchors.centerIn: parent
                        spacing: 5

                        Kirigami.Icon {
                            source: modelData.icon
                            implicitWidth: 12
                            implicitHeight: 12
                            color: Kirigami.Theme.disabledTextColor
                        }

                        Label {
                            text: modelData.label
                            font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.95
                            font.weight: Font.Medium
                            color: Kirigami.Theme.textColor
                        }

                        Label {
                            text: modelData.ext
                            font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.85
                            color: Kirigami.Theme.disabledTextColor
                        }
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        // ---------------------------------------------------------------------
        // 4. Squircle Keypad Grid (Ergonomic Breeze Telephony Keys)
        // ---------------------------------------------------------------------
        GridLayout {
            id: keypadGrid
            Layout.alignment: Qt.AlignHCenter
            columns: 3
            rowSpacing: 12
            columnSpacing: 18

            Repeater {
                model: [
                    { key: "1", sub: "" },
                    { key: "2", sub: "ABC" },
                    { key: "3", sub: "DEF" },
                    { key: "4", sub: "GHI" },
                    { key: "5", sub: "JKL" },
                    { key: "6", sub: "MNO" },
                    { key: "7", sub: "PQRS" },
                    { key: "8", sub: "TUV" },
                    { key: "9", sub: "WXYZ" },
                    { key: "*", sub: "" },
                    { key: "0", sub: "+" },
                    { key: "#", sub: "" }
                ]

                Item {
                    id: keyItem
                    implicitWidth: 72
                    implicitHeight: 66

                    Rectangle {
                        id: keySquircle
                        anchors.fill: parent
                        radius: 14
                        color: keyMouse.pressed ?
                               Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.22) :
                               (keyMouse.containsMouse ?
                                Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10) :
                                Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05))
                        border.color: keyMouse.containsMouse ?
                                      Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.40) :
                                      Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)
                        border.width: 1

                        // Smooth tactile press compression
                        scale: keyMouse.pressed ? 0.93 : 1.0
                        Behavior on scale {
                            NumberAnimation { duration: 70; easing.type: Easing.OutQuad }
                        }
                        Behavior on color {
                            ColorAnimation { duration: 90 }
                        }

                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 0

                            Label {
                                text: modelData.key
                                font.pointSize: (modelData.key === "*" || modelData.key === "#") ? 24 : 22
                                font.weight: Font.DemiBold
                                font.family: "Noto Sans, Inter, Roboto, sans-serif"
                                color: Kirigami.Theme.textColor
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Label {
                                text: modelData.sub
                                font.pointSize: 7.5
                                font.weight: Font.Medium
                                font.letterSpacing: 1.5
                                color: Kirigami.Theme.disabledTextColor
                                visible: modelData.sub.length > 0
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }
                    }

                    MouseArea {
                        id: keyMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor

                        onClicked: {
                            dialInput.text += modelData.key;
                            dialInput.forceActiveFocus();
                        }

                        onPressAndHold: {
                            if (modelData.key === "0") {
                                dialInput.text += "+";
                                dialInput.forceActiveFocus();
                            }
                        }
                    }

                    Accessible.name: modelData.key + (modelData.sub ? (" " + modelData.sub) : "")
                    Accessible.role: Accessible.Button
                }
            }
        }

        Item { Layout.fillHeight: true }

        // ---------------------------------------------------------------------
        // 5. Action Row: Centered Call Button & Dynamic Backspace (Balanced 3-col)
        // ---------------------------------------------------------------------
        GridLayout {
            Layout.alignment: Qt.AlignHCenter
            columns: 3
            columnSpacing: 18

            // Column 1: Left balance spacer (equal width to key: 72px)
            Item {
                implicitWidth: 72
                implicitHeight: 68
            }

            // Column 2: Center Call Button (Vibrant Breeze Emerald Squircle Disc)
            Item {
                implicitWidth: 72
                implicitHeight: 68

                Rectangle {
                    id: callButton
                    anchors.fill: parent
                    radius: 20

                    readonly property bool canCall: page.bridge && page.bridge.is_registered && dialInput.text.trim().length > 0

                    // Dynamic styling: Vibrant green when actionable, muted outline when empty
                    color: canCall ?
                           (callMouse.pressed ? Qt.darker("#27ae60", 1.15) : (callMouse.containsMouse ? "#2ecc71" : "#27ae60")) :
                           Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)

                    border.color: canCall ?
                                  "transparent" :
                                  Qt.rgba(Kirigami.Theme.disabledTextColor.r, Kirigami.Theme.disabledTextColor.g, Kirigami.Theme.disabledTextColor.b, 0.25)
                    border.width: canCall ? 0 : 1.5

                    // Soft elevation glow when active
                    scale: callMouse.pressed ? 0.93 : 1.0
                    Behavior on scale {
                        NumberAnimation { duration: 70; easing.type: Easing.OutQuad }
                    }
                    Behavior on color {
                        ColorAnimation { duration: 150 }
                    }

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        source: "call-start"
                        width: 30
                        height: 30
                        color: callButton.canCall ? "white" : Kirigami.Theme.disabledTextColor
                    }

                    MouseArea {
                        id: callMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: callButton.canCall ? Qt.PointingHandCursor : Qt.ArrowCursor
                        enabled: callButton.canCall
                        onClicked: {
                            if (dialInput.text.trim().length > 0) {
                                page.startCallRequested(dialInput.text.trim());
                            }
                        }
                        ToolTip.text: (!page.bridge || !page.bridge.is_registered) ?
                                      qsTr("Cannot call: SIP account offline") :
                                      (dialInput.text.trim().length === 0 ? qsTr("Enter a number to call") : qsTr("Call %1 (Enter)").arg(dialInput.text.trim()))
                        ToolTip.visible: containsMouse
                    }

                    Accessible.name: qsTr("Place Call")
                    Accessible.role: Accessible.Button
                }
            }

            // Column 3: Right Backspace Button (Aligned directly under '#' key)
            Item {
                implicitWidth: 72
                implicitHeight: 68

                Rectangle {
                    id: backspaceBtn
                    anchors.centerIn: parent
                    width: 52
                    height: 52
                    radius: 14
                    color: backspaceMouse.pressed ?
                           Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.18) :
                           (backspaceMouse.containsMouse ?
                            Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10) :
                            "transparent")

                    opacity: dialInput.text.length > 0 ? 1.0 : 0.0
                    visible: opacity > 0
                    Behavior on opacity {
                        NumberAnimation { duration: 150 }
                    }

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        source: "edit-clear-locationbar-rtl"
                        width: 24
                        height: 24
                        color: Kirigami.Theme.textColor
                    }

                    MouseArea {
                        id: backspaceMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        enabled: dialInput.text.length > 0

                        onClicked: {
                            if (dialInput.text.length > 0) {
                                dialInput.text = dialInput.text.slice(0, -1);
                            }
                            dialInput.forceActiveFocus();
                        }

                        // Long press to erase entire number
                        onPressAndHold: {
                            dialInput.text = "";
                            dialInput.forceActiveFocus();
                        }

                        ToolTip.text: qsTr("Backspace (Hold to clear)")
                        ToolTip.visible: containsMouse
                    }

                    Accessible.name: qsTr("Backspace")
                    Accessible.role: Accessible.Button
                }
            }
        }

        Item { Layout.preferredHeight: 8 }
    }

    // -------------------------------------------------------------------------
    // Audio Quick Settings Sheet (Instant One-Click Audio Device Switching)
    // -------------------------------------------------------------------------
    Kirigami.OverlaySheet {
        id: audioQuickSheet
        title: qsTr("Audio Quick Settings")
        parent: applicationWindow() ? applicationWindow().overlay : null
        implicitHeight: Math.min(
            parent ? parent.height - Kirigami.Units.gridUnit * 4 : 520,
            audioQuickLayout.implicitHeight + Kirigami.Units.gridUnit * 5
        )

        ColumnLayout {
            id: audioQuickLayout
            spacing: Kirigami.Units.largeSpacing
            implicitWidth: Math.min(page.width - (Kirigami.Units.gridUnit * 2), Kirigami.Units.gridUnit * 24)

            // Output device selector
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                RowLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Kirigami.Icon {
                        source: "audio-volume-high"
                        implicitWidth: 20
                        implicitHeight: 20
                        color: Kirigami.Theme.highlightColor
                    }
                    Label {
                        text: qsTr("Speaker / Headset Output:")
                        font.bold: true
                    }
                }

                ComboBox {
                    id: quickOutputSelector
                    Layout.fillWidth: true
                    textRole: "name"
                    valueRole: "id"
                    model: (page.audioDevices.outputs && page.audioDevices.outputs.length > 0) ?
                           page.audioDevices.outputs :
                           [{ id: "default", name: qsTr("Default System Output") }]
                    Component.onCompleted: syncIndex()
                    onModelChanged: syncIndex()
                    function syncIndex() {
                        var curId = page.audioConfig.output_device_id || "default";
                        for (var i = 0; i < count; i++) {
                            if (model[i].id === curId) {
                                currentIndex = i;
                                return;
                            }
                        }
                    }
                    onActivated: {
                        applyAudioConfig();
                    }
                }
            }

            // Input device selector
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                RowLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Kirigami.Icon {
                        source: "audio-input-microphone"
                        implicitWidth: 20
                        implicitHeight: 20
                        color: Kirigami.Theme.highlightColor
                    }
                    Label {
                        text: qsTr("Microphone Input:")
                        font.bold: true
                    }
                }

                ComboBox {
                    id: quickInputSelector
                    Layout.fillWidth: true
                    textRole: "name"
                    valueRole: "id"
                    model: (page.audioDevices.inputs && page.audioDevices.inputs.length > 0) ?
                           page.audioDevices.inputs :
                           [{ id: "default", name: qsTr("Default System Microphone") }]
                    Component.onCompleted: syncIndex()
                    onModelChanged: syncIndex()
                    function syncIndex() {
                        var curId = page.audioConfig.input_device_id || "default";
                        for (var i = 0; i < count; i++) {
                            if (model[i].id === curId) {
                                currentIndex = i;
                                return;
                            }
                        }
                    }
                    onActivated: {
                        applyAudioConfig();
                    }
                }
            }

            Kirigami.Separator { Layout.fillWidth: true }

            // Volume & Silent Mode Quick Controls
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Label {
                    text: qsTr("Ringer Volume:")
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                }

                Slider {
                    id: quickVolumeSlider
                    Layout.fillWidth: true
                    from: 0
                    to: 100
                    value: page.audioConfig.ringer_volume !== undefined ? page.audioConfig.ringer_volume : 80
                    onMoved: {
                        applyAudioConfig();
                    }
                }

                Label {
                    text: Math.round(quickVolumeSlider.value) + "%"
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 2
                    horizontalAlignment: Text.AlignRight
                }
            }

            // Action Buttons (Test sound, Close)
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Button {
                    id: testSoundBtn
                    property bool testSoundPlaying: false
                    text: testSoundPlaying ? qsTr("Stop Test") : qsTr("Test Sound")
                    icon.name: testSoundPlaying ? "media-playback-stop" : "media-playback-start"
                    onClicked: {
                        if (!page.bridge) return;
                        if (testSoundPlaying) {
                            page.bridge.stop_test_ringtone();
                            testSoundPlaying = false;
                        } else {
                            var r = page.audioConfig.ringtone || "standard";
                            var p = page.audioConfig.custom_ringtone_path || "";
                            page.bridge.test_ringtone(r, p, quickVolumeSlider.value);
                            testSoundPlaying = true;
                        }
                    }
                }

                Item { Layout.fillWidth: true }

                Button {
                    text: qsTr("Done")
                    icon.name: "dialog-ok"
                    highlighted: true
                    onClicked: {
                        if (testSoundBtn.testSoundPlaying && page.bridge) {
                            page.bridge.stop_test_ringtone();
                            testSoundBtn.testSoundPlaying = false;
                        }
                        audioQuickSheet.close();
                    }
                }
            }
        }

        function applyAudioConfig() {
            if (!page.bridge) return;
            var outId = quickOutputSelector.currentValue || "default";
            var inId = quickInputSelector.currentValue || "default";
            var ringId = page.audioConfig.ringing_device_id || "default";
            var isSil = page.isSilent;
            var r = page.audioConfig.ringtone || "standard";
            var cp = page.audioConfig.custom_ringtone_path || "";
            var vol = Math.round(quickVolumeSlider.value);
            page.bridge.save_audio_config(inId, outId, ringId, isSil, r, cp, vol);
        }
    }
}
