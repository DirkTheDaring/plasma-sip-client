use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("org.kde.plasma_sip").qml_files([
        "qml/main.qml",
        "qml/DialPadView.qml",
        "qml/ActiveCallView.qml",
        "qml/CallHistoryView.qml",
        "qml/AudioSettingsView.qml",
        "qml/AccountWizardView.qml",
        "qml/IncomingCallBanner.qml",
    ]))
    .qt_module("Network")
    .qt_module("Gui")
    .qt_module("Qml")
    .qt_module("Quick")
    .file("src/bridge.rs")
    .cpp_file("src/desktop_integration.cpp")
    .build();
}
