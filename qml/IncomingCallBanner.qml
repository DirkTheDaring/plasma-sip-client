import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Rectangle {
    id: banner
    anchors.fill: parent
    color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.88)
    z: 999
    visible: false

    property string caller: ""
    property bool isSilent: false

    signal accepted()
    signal rejected()

    focus: visible

    Keys.onReturnPressed: banner.accepted()
    Keys.onEnterPressed: banner.accepted()
    Keys.onEscapePressed: banner.rejected()

    onVisibleChanged: {
        if (visible) {
            banner.forceActiveFocus();
        }
    }

    // Modal incoming call card
    Rectangle {
        id: card
        width: Math.min(parent.width - Kirigami.Units.largeSpacing * 2, 420)
        height: contentCol.implicitHeight + Kirigami.Units.largeSpacing * 4
        anchors.centerIn: parent
        radius: Kirigami.Units.smallSpacing * 3
        color: Kirigami.Theme.backgroundColor
        border.color: Kirigami.Theme.positiveTextColor
        border.width: 2

        ColumnLayout {
            id: contentCol
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing * 1.5
            spacing: Kirigami.Units.largeSpacing

            // Pulsing Ringing Icon & Aura
            Item {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 84
                Layout.preferredHeight: 84

                Rectangle {
                    anchors.centerIn: parent
                    width: 80
                    height: 80
                    radius: 40
                    color: Qt.rgba(Kirigami.Theme.positiveTextColor.r, Kirigami.Theme.positiveTextColor.g, Kirigami.Theme.positiveTextColor.b, 0.2)

                    SequentialAnimation on scale {
                        loops: Animation.Infinite
                        running: banner.visible
                        PropertyAnimation { to: 1.25; duration: 800; easing.type: Easing.OutQuad }
                        PropertyAnimation { to: 1.0; duration: 800; easing.type: Easing.InOutQuad }
                    }
                    SequentialAnimation on opacity {
                        loops: Animation.Infinite
                        running: banner.visible
                        PropertyAnimation { to: 0.1; duration: 800 }
                        PropertyAnimation { to: 0.5; duration: 800 }
                    }
                }

                Rectangle {
                    anchors.centerIn: parent
                    width: 64
                    height: 64
                    radius: 32
                    color: Kirigami.Theme.positiveTextColor

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        source: "call-start"
                        width: 36
                        height: 36
                        color: "white"
                    }
                }
            }

            // Caller Information Header
            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignHCenter
                spacing: Kirigami.Units.smallSpacing

                Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: qsTr("INCOMING CALL")
                    font.pointSize: 11
                    font.bold: true
                    font.capitalization: Font.AllUppercase
                    color: Kirigami.Theme.positiveTextColor
                }

                Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: banner.caller !== "" ? banner.caller : qsTr("Unknown Caller")
                    font.pointSize: 20
                    font.bold: true
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: qsTr("Press Enter to Answer • Esc to Decline")
                    font.pointSize: 9
                    color: Kirigami.Theme.disabledTextColor
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: Kirigami.Units.smallSpacing
                    visible: banner.isSilent

                    Kirigami.Icon {
                        source: "audio-volume-muted"
                        implicitWidth: 16
                        implicitHeight: 16
                        color: Kirigami.Theme.neutralTextColor
                    }

                    Label {
                        text: qsTr("Ringtone Muted (Silent Mode)")
                        font.pointSize: 9
                        font.bold: true
                        color: Kirigami.Theme.neutralTextColor
                    }
                }
            }

            Item { Layout.preferredHeight: Kirigami.Units.smallSpacing }

            // Action Buttons: Answer and Decline
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing

                // Answer Button
                Button {
                    id: acceptButton
                    Layout.fillWidth: true
                    Layout.preferredHeight: 52
                    icon.name: "call-start"
                    text: qsTr("Answer (Enter)")
                    font.bold: true
                    font.pointSize: 12
                    scale: down ? 0.96 : 1.0
                    Behavior on scale { NumberAnimation { duration: 60 } }
                    Accessible.name: qsTr("Answer Incoming Call")
                    Accessible.role: Accessible.Button

                    background: Rectangle {
                        radius: Kirigami.Units.smallSpacing * 2
                        color: acceptButton.down ? Qt.darker(Kirigami.Theme.positiveTextColor, 1.2) : Kirigami.Theme.positiveTextColor
                    }
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.smallSpacing
                        Kirigami.Icon {
                            source: "call-start"
                            Layout.preferredWidth: 24
                            Layout.preferredHeight: 24
                            color: "white"
                        }
                        Label {
                            text: acceptButton.text
                            font: acceptButton.font
                            color: "white"
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }
                    }
                    onClicked: banner.accepted()
                }

                // Decline Button
                Button {
                    id: declineButton
                    Layout.fillWidth: true
                    Layout.preferredHeight: 52
                    icon.name: "call-stop"
                    text: qsTr("Decline (Esc)")
                    font.bold: true
                    font.pointSize: 12
                    scale: down ? 0.96 : 1.0
                    Behavior on scale { NumberAnimation { duration: 60 } }
                    Accessible.name: qsTr("Decline Incoming Call")
                    Accessible.role: Accessible.Button

                    background: Rectangle {
                        radius: Kirigami.Units.smallSpacing * 2
                        color: declineButton.down ? Qt.darker(Kirigami.Theme.negativeTextColor, 1.2) : Kirigami.Theme.negativeTextColor
                    }
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.smallSpacing
                        Kirigami.Icon {
                            source: "call-stop"
                            Layout.preferredWidth: 24
                            Layout.preferredHeight: 24
                            color: "white"
                        }
                        Label {
                            text: declineButton.text
                            font: declineButton.font
                            color: "white"
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }
                    }
                    onClicked: banner.rejected()
                }
            }
        }
    }
}
