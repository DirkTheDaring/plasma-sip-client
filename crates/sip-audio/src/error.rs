use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("No audio device available")]
    NoDeviceAvailable,
    #[error("Audio device not found: {0}")]
    DeviceNotFound(String),
    #[error("Failed to build audio stream: {0}")]
    StreamBuild(String),
    #[error("Failed to start audio stream: {0}")]
    StreamPlay(String),
    #[error("Audio channel/queue error: {0}")]
    Channel(String),
    #[error("Unsupported audio sample format: {0:?}")]
    UnsupportedFormat(cpal::SampleFormat),
}
