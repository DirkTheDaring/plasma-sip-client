use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    Pcma,           // G.711 A-law (RTP payload 8)
    Pcmu,           // G.711 μ-law (RTP payload 0)
    TelephoneEvent, // RFC 4733 DTMF (typically payload 101)
}

impl AudioCodec {
    pub fn payload_type(&self) -> u8 {
        match self {
            Self::Pcmu => 0,
            Self::Pcma => 8,
            Self::TelephoneEvent => 101,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        8000
    }

    pub fn channels(&self) -> u16 {
        1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    #[serde(default)]
    pub ringing_device_id: Option<String>,
    #[serde(default)]
    pub silent_mode: bool,
    #[serde(default = "default_ringtone")]
    pub ringtone: String,
    #[serde(default)]
    pub custom_ringtone_path: Option<String>,
    #[serde(default = "default_ringer_volume")]
    pub ringer_volume: u32,
    pub preferred_codecs: Vec<AudioCodec>,
    pub jitter_buffer_target_ms: u32,
    pub echo_cancellation: bool,
}

fn default_ringtone() -> String {
    "marimba".to_string()
}

fn default_ringer_volume() -> u32 {
    80
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device_id: None,
            output_device_id: None,
            ringing_device_id: None,
            silent_mode: false,
            ringtone: default_ringtone(),
            custom_ringtone_path: None,
            ringer_volume: default_ringer_volume(),
            preferred_codecs: vec![AudioCodec::Pcma, AudioCodec::Pcmu],
            jitter_buffer_target_ms: 60,
            echo_cancellation: true,
        }
    }
}
