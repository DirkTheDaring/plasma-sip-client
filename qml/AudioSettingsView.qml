import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    title: qsTr("Audio & Devices")

    property var bridge: null
    property bool isTestingRingtone: false

    property var devicesData: {
        if (!page.bridge || !page.bridge.audio_devices_json) return { inputs: [], outputs: [] };
        try {
            var parsed = JSON.parse(page.bridge.audio_devices_json);
            return parsed && typeof parsed === "object" ? parsed : { inputs: [], outputs: [] };
        } catch (e) {
            return { inputs: [], outputs: [] };
        }
    }

    property var audioConfig: {
        if (!page.bridge || !page.bridge.audio_config_json) return {};
        try {
            var parsed = JSON.parse(page.bridge.audio_config_json);
            return parsed && typeof parsed === "object" ? parsed : {};
        } catch (e) {
            return {};
        }
    }

    property var inputModel: {
        var list = [
            { id: "default", text: qsTr("Default System Capture (PipeWire)") }
        ];
        var rawInputs = (page.devicesData && page.devicesData.inputs) ? page.devicesData.inputs : [];
        for (var i = 0; i < rawInputs.length; i++) {
            var d = rawInputs[i];
            if (d && d.name) {
                list.push({
                    id: d.id || d.name,
                    text: d.name + (d.is_default ? " (" + qsTr("Default") + ")" : "")
                });
            }
        }
        return list;
    }

    property var outputModel: {
        var list = [
            { id: "default", text: qsTr("Default System Playback (PipeWire)") }
        ];
        var rawOutputs = (page.devicesData && page.devicesData.outputs) ? page.devicesData.outputs : [];
        for (var i = 0; i < rawOutputs.length; i++) {
            var d = rawOutputs[i];
            if (d && d.name) {
                list.push({
                    id: d.id || d.name,
                    text: d.name + (d.is_default ? " (" + qsTr("Default") + ")" : "")
                });
            }
        }
        return list;
    }

    property var ringtoneModel: [
        { id: "marimba", text: qsTr("Marimba Chime (Modern & Pleasant)") },
        { id: "ocean", text: qsTr("KDE Ocean (System Theme)") },
        { id: "classic_bell", text: qsTr("Classic Telephone Bell") },
        { id: "acoustic", text: qsTr("Soft Acoustic Chime") },
        { id: "standard_tone", text: qsTr("Standard PBX Progress Tone (425 Hz)") },
        { id: "custom", text: qsTr("Custom Audio File...") }
    ]

    function findIndex(model, targetId) {
        if (!targetId || targetId === "default") return 0;
        for (var i = 0; i < model.length; i++) {
            if (model[i].id === targetId || model[i].text.indexOf(targetId) !== -1) {
                return i;
            }
        }
        return 0;
    }

    function findRingtoneIndex(ringtoneId) {
        if (!ringtoneId) return 0;
        for (var i = 0; i < ringtoneModel.length; i++) {
            if (ringtoneModel[i].id === ringtoneId) {
                return i;
            }
        }
        return 0;
    }

    function syncUiFromConfig() {
        if (!page.audioConfig) return;
        if (inputCombo && page.inputModel.length > 0) {
            inputCombo.currentIndex = page.findIndex(page.inputModel, page.audioConfig.input_device_id);
        }
        if (outputCombo && page.outputModel.length > 0) {
            outputCombo.currentIndex = page.findIndex(page.outputModel, page.audioConfig.output_device_id);
        }
        if (ringingCombo && page.outputModel.length > 0) {
            ringingCombo.currentIndex = page.findIndex(page.outputModel, page.audioConfig.ringing_device_id);
        }
        if (silentModeSwitch) {
            silentModeSwitch.checked = (page.audioConfig.silent_mode === true);
        }
        if (ringtoneCombo) {
            var curRingtone = page.audioConfig.ringtone || "marimba";
            ringtoneCombo.currentIndex = page.findRingtoneIndex(curRingtone);
        }
        if (customPathField && page.audioConfig.custom_ringtone_path !== undefined) {
            customPathField.text = page.audioConfig.custom_ringtone_path || "";
        }
        if (ringerVolumeSlider && page.audioConfig.ringer_volume !== undefined) {
            ringerVolumeSlider.value = page.audioConfig.ringer_volume;
        }
    }

    onAudioConfigChanged: syncUiFromConfig()
    onOutputModelChanged: syncUiFromConfig()
    onInputModelChanged: syncUiFromConfig()
    Component.onCompleted: syncUiFromConfig()

    Component.onDestruction: {
        if (page.isTestingRingtone && page.bridge) {
            page.bridge.stop_test_ringtone();
        }
    }

    Timer {
        id: testTimer
        interval: 6200
        repeat: false
        onTriggered: page.isTestingRingtone = false
    }

    function saveCurrentSettings() {
        if (!page.bridge) return;
        var inId = inputCombo.currentIndex > 0 && inputCombo.currentIndex < page.inputModel.length
            ? page.inputModel[inputCombo.currentIndex].id : "default";
        var outId = outputCombo.currentIndex > 0 && outputCombo.currentIndex < page.outputModel.length
            ? page.outputModel[outputCombo.currentIndex].id : "default";
        var ringDevId = ringingCombo.currentIndex > 0 && ringingCombo.currentIndex < page.outputModel.length
            ? page.outputModel[ringingCombo.currentIndex].id : "default";
        var silent = silentModeSwitch.checked;
        var ringtoneId = ringtoneCombo.currentIndex >= 0 && ringtoneCombo.currentIndex < page.ringtoneModel.length
            ? page.ringtoneModel[ringtoneCombo.currentIndex].id : "marimba";
        var customPath = customPathField ? customPathField.text.trim() : "";
        var ringerVol = ringerVolumeSlider ? Math.round(ringerVolumeSlider.value) : 80;

        page.bridge.save_audio_config(inId, outId, ringDevId, silent, ringtoneId, customPath, ringerVol);
        saveFeedback.text = qsTr("Audio preferences saved successfully.");
        saveFeedback.visible = true;
        feedbackTimer.restart();
    }

    Timer {
        id: feedbackTimer
        interval: 3000
        repeat: false
        onTriggered: saveFeedback.visible = false
    }

    FileDialog {
        id: ringtoneFileDialog
        title: qsTr("Select Ringtone Audio File")
        nameFilters: [
            qsTr("Audio files (*.wav *.ogg *.oga *.mp3 *.flac *.opus *.m4a)"),
            qsTr("WAV files (*.wav)"),
            qsTr("OGG audio (*.ogg *.oga)"),
            qsTr("All files (*)")
        ]
        onAccepted: {
            var path = selectedFile.toString();
            if (path.indexOf("file://") === 0) {
                path = path.substring(7);
            }
            customPathField.text = decodeURIComponent(path);
            ringtoneCombo.currentIndex = page.findRingtoneIndex("custom");
            page.saveCurrentSettings();
        }
    }

    actions: [
        Kirigami.Action {
            text: qsTr("Refresh Devices")
            icon.name: "view-refresh"
            checkable: false
            onTriggered: {
                if (page.bridge) page.bridge.refresh_devices();
            }
        }
    ]

    Kirigami.FormLayout {
        width: parent.width

        Kirigami.InlineMessage {
            id: saveFeedback
            Layout.fillWidth: true
            type: Kirigami.MessageType.Positive
            text: qsTr("Audio device preferences saved successfully.")
            visible: false
            showCloseButton: true
        }

        // Section 1: Call Communication Audio (Headset / Earpiece)
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Call Communication Audio")
        }

        Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            color: Kirigami.Theme.disabledTextColor
            text: qsTr("Audio devices used for active two-way phone calls (e.g. your Jabra headset). If the device is unplugged or unavailable, calls automatically fall back to the default system audio.")
        }

        ComboBox {
            id: outputCombo
            Kirigami.FormData.label: qsTr("Call Audio Output:")
            Layout.fillWidth: true
            textRole: "text"
            valueRole: "id"
            model: page.outputModel
            currentIndex: page.findIndex(page.outputModel, page.audioConfig.output_device_id)
            onActivated: page.saveCurrentSettings()
        }

        ComboBox {
            id: inputCombo
            Kirigami.FormData.label: qsTr("Microphone Input:")
            Layout.fillWidth: true
            textRole: "text"
            valueRole: "id"
            model: page.inputModel
            currentIndex: page.findIndex(page.inputModel, page.audioConfig.input_device_id)
            onActivated: page.saveCurrentSettings()
        }

        // Section 2: Ringtone & Incoming Alerts (Desktop Speakers)
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Ringtone & Incoming Alerts")
        }

        Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            color: Kirigami.Theme.disabledTextColor
            text: qsTr("Device used to sound the incoming call ringing alert. Select your desktop speakers so you hear calls clearly when working at your PC without wearing your headset.")
        }

        ComboBox {
            id: ringingCombo
            Kirigami.FormData.label: qsTr("Ringtone Output:")
            Layout.fillWidth: true
            textRole: "text"
            valueRole: "id"
            model: page.outputModel
            currentIndex: page.findIndex(page.outputModel, page.audioConfig.ringing_device_id)
            onActivated: page.saveCurrentSettings()
        }

        ComboBox {
            id: ringtoneCombo
            Kirigami.FormData.label: qsTr("Ringtone Sound:")
            Layout.fillWidth: true
            textRole: "text"
            valueRole: "id"
            model: page.ringtoneModel
            currentIndex: page.findRingtoneIndex(page.audioConfig.ringtone || "marimba")
            onActivated: page.saveCurrentSettings()
        }

        RowLayout {
            id: customFileRow
            Kirigami.FormData.label: qsTr("Custom Sound File:")
            Layout.fillWidth: true
            visible: ringtoneCombo.currentIndex === page.findRingtoneIndex("custom")
            spacing: Kirigami.Units.smallSpacing

            TextField {
                id: customPathField
                Layout.fillWidth: true
                placeholderText: qsTr("/path/to/custom_ringtone.wav, .ogg, .mp3...")
                text: page.audioConfig.custom_ringtone_path || ""
                onEditingFinished: page.saveCurrentSettings()
            }

            Button {
                text: qsTr("Browse…")
                icon.name: "document-open"
                onClicked: ringtoneFileDialog.open()
            }
        }

        Label {
            visible: ringtoneCombo.currentIndex === page.findRingtoneIndex("custom")
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            color: Kirigami.Theme.disabledTextColor
            text: qsTr("Supports .wav, .ogg, .oga, .mp3, .flac, .opus audio files.")
        }

        RowLayout {
            Kirigami.FormData.label: qsTr("Ringer Volume:")
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            Slider {
                id: ringerVolumeSlider
                Layout.fillWidth: true
                from: 0
                to: 100
                stepSize: 1
                value: page.audioConfig.ringer_volume !== undefined ? page.audioConfig.ringer_volume : 80
                onMoved: page.saveCurrentSettings()
            }

            Label {
                text: Math.round(ringerVolumeSlider.value) + "%"
                Layout.preferredWidth: 40
                horizontalAlignment: Text.AlignRight
                font.bold: true
            }
        }

        RowLayout {
            Kirigami.FormData.label: qsTr("Preview:")
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            Button {
                id: testRingtoneBtn
                text: page.isTestingRingtone ? qsTr("Stop Preview") : qsTr("Test Ringtone")
                icon.name: page.isTestingRingtone ? "media-playback-stop" : "media-playback-start"
                onClicked: {
                    if (page.isTestingRingtone) {
                        if (page.bridge) page.bridge.stop_test_ringtone();
                        page.isTestingRingtone = false;
                        testTimer.stop();
                    } else {
                        var ringId = page.ringtoneModel[ringtoneCombo.currentIndex].id;
                        var customPath = customPathField ? customPathField.text.trim() : "";
                        var vol = ringerVolumeSlider ? Math.round(ringerVolumeSlider.value) : 80;
                        if (page.bridge) page.bridge.test_ringtone(ringId, customPath, vol);
                        page.isTestingRingtone = true;
                        testTimer.restart();
                    }
                }
            }

            Label {
                Layout.fillWidth: true
                text: page.isTestingRingtone
                    ? qsTr("Playing preview on %1…").arg(ringingCombo.currentText)
                    : qsTr("Plays preview on the selected Ringtone Output device")
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                color: page.isTestingRingtone ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }

        CheckBox {
            id: silentModeSwitch
            Kirigami.FormData.label: qsTr("Silent Mode:")
            text: qsTr("Mute ringtone (Do Not Disturb)")
            checked: page.audioConfig.silent_mode === true
            onToggled: page.saveCurrentSettings()
        }

        Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            color: silentModeSwitch.checked ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.disabledTextColor
            text: silentModeSwitch.checked
                ? qsTr("Silent mode active: Incoming calls will display visual desktop notifications without playing an audible ringtone.")
                : qsTr("When silent mode is enabled, ringtone sound is muted while visual desktop notifications remain active.")
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Codec Preferences")
        }

        ComboBox {
            Kirigami.FormData.label: qsTr("Preferred Codec:")
            Layout.fillWidth: true
            model: [
                "G.711 A-law (PCMA) - Standard European / FRITZ!Box",
                "G.711 μ-law (PCMU) - North American"
            ]
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: qsTr("Call Rejection Behavior")
        }

        ComboBox {
            id: rejectModeCombo
            Kirigami.FormData.label: qsTr("When declining call:")
            Layout.fillWidth: true
            textRole: "text"
            valueRole: "id"
            model: [
                { id: "603_decline", text: qsTr("603 Decline (Default - standard rejection)") },
                { id: "486_busy_here", text: qsTr("486 Busy Here (Signals busy line)") },
                { id: "600_busy_everywhere", text: qsTr("600 Busy Everywhere (Signals all lines busy)") },
                { id: "answer_and_drop", text: qsTr("Answer & Drop (Forceful hangup - stops other phones)") }
            ]
            currentIndex: {
                var cur = (page.bridge && page.bridge.reject_mode) ? page.bridge.reject_mode : "603_decline";
                for (var i = 0; i < model.length; i++) {
                    if (model[i].id === cur) return i;
                }
                return 0;
            }
            onActivated: {
                if (page.bridge) {
                    var selectedId = model[currentIndex].id;
                    page.bridge.change_reject_mode(selectedId);
                    saveFeedback.text = qsTr("Call reject mode updated to: ") + model[currentIndex].text;
                    saveFeedback.visible = true;
                    feedbackTimer.restart();
                }
            }
        }

        Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            color: Kirigami.Theme.disabledTextColor
            text: {
                switch (rejectModeCombo.currentIndex) {
                    case 0:
                        return qsTr("Standard SIP 603 Decline. If other extensions or answering machine are in the FRITZ!Box ring group, they may continue ringing.");
                    case 1:
                        return qsTr("SIP 486 Busy Here. Informs the PBX that this extension is busy.");
                    case 2:
                        return qsTr("SIP 600 Busy Everywhere. Informs the PBX that all extensions are busy.");
                    case 3:
                        return qsTr("Forceful disconnect: Briefly accepts call (200 OK) and immediately sends BYE, cancelling ringing on all other phones in the FRITZ!Box ring group. Note: Because the call connects, callers without an all-net flatrate may incur connection charges (typically the 1st minute).");
                    default:
                        return "";
                }
            }
        }
    }
}
