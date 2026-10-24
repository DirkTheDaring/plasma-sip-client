import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Call in Progress")
    focus: true

    property var bridge: null
    property string remoteParty: ""
    property string callState: "Active"
    property int durationSeconds: 0
    property int elapsedSeconds: 0
    property bool isMuted: false

    onDurationSecondsChanged: {
        elapsedSeconds = durationSeconds;
    }
    Component.onCompleted: {
        elapsedSeconds = durationSeconds;
    }

    signal hangupRequested()
    signal toggleMuteRequested()
    signal sendDtmfRequested(string digit)
    signal holdRequested()
    signal resumeRequested()

    Keys.onPressed: function(event) {
        if (event.key === Qt.Key_H) {
            if (page.callState === "Held") {
                page.resumeRequested();
            } else {
                page.holdRequested();
            }
            event.accepted = true;
            return;
        }
        if (event.key === Qt.Key_M || event.key === Qt.Key_Space) {
            page.toggleMuteRequested();
            event.accepted = true;
            return;
        }
        if (event.key === Qt.Key_Escape) {
            page.hangupRequested();
            event.accepted = true;
            return;
        }
        if (event.text.length > 0) {
            var validChars = "0123456789*#";
            if (validChars.indexOf(event.text) !== -1) {
                page.sendDtmfRequested(event.text);
                event.accepted = true;
                return;
            }
        }
    }

    function formatTime(secs) {
        var m = Math.floor(secs / 60);
        var s = secs % 60;
        return (m < 10 ? "0" : "") + m + ":" + (s < 10 ? "0" : "") + s;
    }

    Timer {
        id: durationTimer
        interval: 1000
        repeat: true
        running: page.callState === "Active"
        onTriggered: {
            page.elapsedSeconds += 1;
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.gridUnit * 2
        spacing: Kirigami.Units.largeSpacing

        Item { Layout.fillHeight: true }

        // Avatar with subtle voice activity aura
        Item {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: 112
            Layout.preferredHeight: 112

            // Pulsing voice activity aura when talking and not muted
            Rectangle {
                anchors.centerIn: parent
                width: 112
                height: 112
                radius: 56
                color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.25)
                visible: page.callState === "Active" && !page.isMuted

                SequentialAnimation on scale {
                    loops: Animation.Infinite
                    running: page.callState === "Active" && !page.isMuted
                    PropertyAnimation { to: 1.15; duration: 1000; easing.type: Easing.OutQuad }
                    PropertyAnimation { to: 1.0; duration: 1000; easing.type: Easing.InOutQuad }
                }
            }

            Rectangle {
                anchors.centerIn: parent
                width: 96
                height: 96
                radius: 48
                color: page.isMuted ? Kirigami.Theme.negativeBackgroundColor : Kirigami.Theme.highlightColor
                border.color: page.isMuted ? Kirigami.Theme.negativeTextColor : "transparent"
                border.width: page.isMuted ? 2 : 0

                Kirigami.Icon {
                    anchors.centerIn: parent
                    width: 48
                    height: 48
                    source: page.isMuted ? "audio-volume-muted" : "user-identity"
                    color: page.isMuted ? Kirigami.Theme.negativeTextColor : "white"
                }
            }
        }

        // Remote Party Name / Number
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: page.remoteParty !== "" ? page.remoteParty : qsTr("Active Call")
            font.pointSize: 24
            font.bold: true
        }

        // Call State & Timer
        ColumnLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: 4

            Label {
                Layout.alignment: Qt.AlignHCenter
                text: page.callState === "Active" ? formatTime(page.elapsedSeconds) : page.callState
                font.pointSize: 18
                font.bold: true
                color: page.callState === "Active" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.neutralTextColor
            }

            Label {
                Layout.alignment: Qt.AlignHCenter
                text: page.isMuted ? qsTr("Microphone Muted (Space / M to unmute)") : qsTr("G.711 A-law • PipeWire (20ms)")
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                color: page.isMuted ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.disabledTextColor
            }
        }

        Item { Layout.fillHeight: true }

        // Mid-call action buttons: Mute, Keypad
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.gridUnit * 2

            // Mute Button with distinct muted alert state
            Button {
                id: muteBtn
                implicitWidth: 64
                implicitHeight: 64
                icon.name: page.isMuted ? "audio-volume-muted" : "audio-input-microphone"
                checkable: true
                checked: page.isMuted
                scale: down ? 0.94 : 1.0
                Behavior on scale { NumberAnimation { duration: 60 } }
                onClicked: page.toggleMuteRequested()

                ToolTip.text: page.isMuted ? qsTr("Unmute Microphone (Space / M)") : qsTr("Mute Microphone (Space / M)")
                ToolTip.visible: hovered
                Accessible.name: ToolTip.text
                Accessible.role: Accessible.Button

                background: Rectangle {
                    radius: Kirigami.Units.smallSpacing * 2
                    color: page.isMuted ?
                           Kirigami.Theme.negativeBackgroundColor :
                           (muteBtn.down ? Kirigami.Theme.highlightColor : (muteBtn.hovered ? Kirigami.Theme.hoverColor : Kirigami.Theme.backgroundColor))
                    border.color: page.isMuted ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.disabledTextColor
                    border.width: page.isMuted ? 2 : 1
                }
            }

            // Hold / Resume Button
            Button {
                id: holdBtn
                implicitWidth: 64
                implicitHeight: 64
                icon.name: page.callState === "Held" ? "media-playback-start" : "media-playback-pause"
                checkable: true
                checked: page.callState === "Held"
                scale: down ? 0.94 : 1.0
                Behavior on scale { NumberAnimation { duration: 60 } }
                onClicked: {
                    if (page.callState === "Held") {
                        page.resumeRequested();
                    } else {
                        page.holdRequested();
                    }
                }

                ToolTip.text: page.callState === "Held" ? qsTr("Resume Call (H)") : qsTr("Hold Call (H)")
                ToolTip.visible: hovered
                Accessible.name: ToolTip.text
                Accessible.role: Accessible.Button

                background: Rectangle {
                    radius: Kirigami.Units.smallSpacing * 2
                    color: page.callState === "Held" ?
                           Kirigami.Theme.neutralBackgroundColor :
                           (holdBtn.down ? Kirigami.Theme.highlightColor : (holdBtn.hovered ? Kirigami.Theme.hoverColor : Kirigami.Theme.backgroundColor))
                    border.color: page.callState === "Held" ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.disabledTextColor
                    border.width: page.callState === "Held" ? 2 : 1
                }
            }

            // In-call DTMF Keypad Button
            Button {
                id: keypadToggleBtn
                implicitWidth: 64
                implicitHeight: 64
                icon.name: "input-dialpad"
                checkable: true
                scale: down ? 0.94 : 1.0
                Behavior on scale { NumberAnimation { duration: 60 } }

                ToolTip.text: qsTr("Show In-Call Keypad (0-9, *, #)")
                ToolTip.visible: hovered
                Accessible.name: ToolTip.text
                Accessible.role: Accessible.Button
            }
        }

        // Collapsible In-call DTMF Keypad
        GridLayout {
            id: inCallKeypad
            visible: keypadToggleBtn.checked
            Layout.alignment: Qt.AlignHCenter
            columns: 3
            rowSpacing: Kirigami.Units.smallSpacing
            columnSpacing: Kirigami.Units.smallSpacing

            Repeater {
                model: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"]
                Button {
                    implicitWidth: 52
                    implicitHeight: 52
                    text: modelData
                    font.pointSize: 15
                    font.bold: true
                    scale: down ? 0.94 : 1.0
                    Behavior on scale { NumberAnimation { duration: 60 } }
                    Accessible.name: qsTr("DTMF tone ") + modelData
                    Accessible.role: Accessible.Button
                    onClicked: {
                        page.sendDtmfRequested(modelData);
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        // End Call Button with semantic theme coloring & tactile feel
        Button {
            id: hangupBtn
            Layout.alignment: Qt.AlignHCenter
            implicitWidth: 80
            implicitHeight: 80
            icon.name: "call-stop"
            icon.width: 32
            icon.height: 32
            scale: down ? 0.94 : 1.0
            Behavior on scale { NumberAnimation { duration: 60 } }

            ToolTip.text: qsTr("End Call (Esc)")
            ToolTip.visible: hovered
            Accessible.name: qsTr("End Call")
            Accessible.role: Accessible.Button

            background: Rectangle {
                radius: 40
                color: hangupBtn.down ?
                       Qt.darker(Kirigami.Theme.negativeTextColor, 1.2) :
                       Kirigami.Theme.negativeTextColor
            }

            contentItem: Item {
                Kirigami.Icon {
                    anchors.centerIn: parent
                    source: "call-stop"
                    width: 32
                    height: 32
                    color: "white"
                }
            }

            onClicked: {
                page.hangupRequested();
            }
        }

        Item { Layout.fillHeight: true }
    }
}
