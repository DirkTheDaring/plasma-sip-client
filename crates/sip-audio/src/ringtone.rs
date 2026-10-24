//! Ringtone synthesis, decoding of custom audio files, and frame generation.

use std::f32::consts::PI;
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

/// Safe categories for ringtone loading errors to prevent exposing user file paths or filenames in logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingtoneLoadErrorKind {
    NotFound,
    UnsupportedFormat,
    DecodeFailed,
    ReadFailed,
}

impl std::fmt::Display for RingtoneLoadErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "File not found"),
            Self::UnsupportedFormat => write!(f, "Unsupported audio format"),
            Self::DecodeFailed => write!(f, "Audio decoding failed"),
            Self::ReadFailed => write!(f, "File read error"),
        }
    }
}

pub const SAMPLE_RATE: u32 = 8000;
pub const FRAME_DURATION_MS: u32 = 20;
pub const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE as usize * FRAME_DURATION_MS as usize) / 1000; // 160

/// Generates a cyclic list of 20ms mono PCM frames (8000 Hz, 160 samples per frame)
/// for the specified ringtone name, optional custom file path, and volume percentage (0..=100).
pub fn generate_ringtone_frames(
    ringtone: &str,
    custom_path: Option<&str>,
    volume_pct: u32,
) -> Vec<Vec<i16>> {
    let vol_factor = (volume_pct.min(100) as f32) / 100.0;

    let raw_samples = match ringtone {
        "custom" => {
            if let Some(path) = custom_path {
                match load_audio_file(path) {
                    Ok(samples) if !samples.is_empty() => samples,
                    Err(err_kind) => {
                        warn!(
                            error_category = %err_kind,
                            "Failed to load custom ringtone, falling back to marimba"
                        );
                        synth_marimba()
                    }
                    _ => synth_marimba(),
                }
            } else {
                synth_marimba()
            }
        }
        "ocean" => {
            let ocean_paths = [
                "/usr/share/sounds/ocean/stereo/phone-incoming-call.oga",
                "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga",
            ];
            let mut loaded = None;
            for p in ocean_paths {
                if Path::new(p).exists() {
                    if let Ok(samples) = load_audio_file(p) {
                        if !samples.is_empty() {
                            loaded = Some(samples);
                            break;
                        }
                    }
                }
            }
            loaded.unwrap_or_else(|| {
                warn!("KDE Ocean ringtone file not found, falling back to marimba");
                synth_marimba()
            })
        }
        "classic_bell" => synth_classic_bell(),
        "acoustic" => synth_acoustic(),
        "standard_tone" => synth_standard_tone(),
        _ => synth_marimba(),
    };

    // Apply volume scaling and slice into 160-sample (20ms) frames
    let scaled_samples: Vec<i16> = raw_samples
        .into_iter()
        .map(|s| ((s as f32) * vol_factor).clamp(i16::MIN as f32, i16::MAX as f32) as i16)
        .collect();

    let mut frames = Vec::new();
    for chunk in scaled_samples.chunks(SAMPLES_PER_FRAME) {
        let mut frame = chunk.to_vec();
        if frame.len() < SAMPLES_PER_FRAME {
            frame.resize(SAMPLES_PER_FRAME, 0);
        }
        frames.push(frame);
    }

    if frames.is_empty() {
        frames.push(vec![0i16; SAMPLES_PER_FRAME]);
    }
    frames
}

/// Attempts to load an audio file and decode it to 8000 Hz mono PCM samples.
/// Uses ffmpeg if available for maximum format compatibility (.oga, .ogg, .mp3, .flac, .wav, .m4a),
/// falling back to a native RIFF/WAVE parser for .wav files.
pub fn load_audio_file(path: &str) -> Result<Vec<i16>, RingtoneLoadErrorKind> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(RingtoneLoadErrorKind::NotFound);
    }

    // Try decoding with ffmpeg
    if let Ok(output) = Command::new("ffmpeg")
        .args([
            "-v", "error", "-i", path, "-f", "s16le", "-ac", "1", "-ar", "8000", "pipe:1",
        ])
        .output()
    {
        if output.status.success() && !output.stdout.is_empty() {
            let mut samples = Vec::with_capacity(output.stdout.len() / 2);
            for chunk in output.stdout.chunks_exact(2) {
                samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
            debug!(samples = samples.len(), "Decoded audio file using ffmpeg");
            return Ok(samples);
        }
    }

    // Fallback: Native PCM WAV parser for .wav files
    if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
        if ext.eq_ignore_ascii_case("wav") {
            return decode_wav_file(p);
        }
        return Err(RingtoneLoadErrorKind::UnsupportedFormat);
    }

    Err(RingtoneLoadErrorKind::DecodeFailed)
}

/// Native RIFF WAV decoder for uncompressed PCM audio files.
fn decode_wav_file(path: &Path) -> Result<Vec<i16>, RingtoneLoadErrorKind> {
    let bytes = std::fs::read(path).map_err(|_| RingtoneLoadErrorKind::ReadFailed)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(RingtoneLoadErrorKind::UnsupportedFormat);
    }

    let mut offset = 12;
    let mut channels: u16 = 1;
    let mut sample_rate: u32 = 44100;
    let mut bits_per_sample: u16 = 16;
    let mut audio_format: u16 = 1;
    let mut pcm_data: Option<&[u8]> = None;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        offset += 8;

        if offset + chunk_size > bytes.len() {
            break;
        }

        if chunk_id == b"fmt " && chunk_size >= 16 {
            audio_format = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
            channels = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
            sample_rate = u32::from_le_bytes([
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ]);
            bits_per_sample = u16::from_le_bytes([bytes[offset + 14], bytes[offset + 15]]);
        } else if chunk_id == b"data" {
            pcm_data = Some(&bytes[offset..offset + chunk_size]);
        }

        offset += chunk_size;
    }

    let raw = pcm_data.ok_or(RingtoneLoadErrorKind::DecodeFailed)?;
    if audio_format != 1 && audio_format != 3 {
        return Err(RingtoneLoadErrorKind::UnsupportedFormat);
    }
    if channels == 0 || sample_rate == 0 {
        return Err(RingtoneLoadErrorKind::DecodeFailed);
    }

    // Convert raw bytes to intermediate mono f32 samples
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    if bytes_per_sample == 0 {
        return Err(RingtoneLoadErrorKind::DecodeFailed);
    }
    let frame_size = bytes_per_sample * channels as usize;
    let total_frames = raw.len() / frame_size;
    let mut mono_samples_native = Vec::with_capacity(total_frames);

    for frame_idx in 0..total_frames {
        let frame_start = frame_idx * frame_size;
        let mut channel_sum = 0.0f32;

        for ch in 0..channels as usize {
            let s_start = frame_start + ch * bytes_per_sample;
            let sample_f32 = match bits_per_sample {
                16 => {
                    let s = i16::from_le_bytes([raw[s_start], raw[s_start + 1]]);
                    s as f32 / 32768.0
                }
                24 => {
                    let s =
                        i32::from_le_bytes([0, raw[s_start], raw[s_start + 1], raw[s_start + 2]])
                            >> 8;
                    s as f32 / 8388608.0
                }
                32 => {
                    if audio_format == 3 {
                        f32::from_le_bytes([
                            raw[s_start],
                            raw[s_start + 1],
                            raw[s_start + 2],
                            raw[s_start + 3],
                        ])
                    } else {
                        let s = i32::from_le_bytes([
                            raw[s_start],
                            raw[s_start + 1],
                            raw[s_start + 2],
                            raw[s_start + 3],
                        ]);
                        s as f32 / 2147483648.0
                    }
                }
                8 => {
                    let s = raw[s_start] as f32;
                    (s - 128.0) / 128.0
                }
                _ => 0.0,
            };
            channel_sum += sample_f32;
        }
        mono_samples_native.push(channel_sum / (channels as f32));
    }

    // Resample mono samples to 8000 Hz if necessary
    if sample_rate == 8000 {
        let res: Vec<i16> = mono_samples_native
            .into_iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        return Ok(res);
    }

    let ratio = 8000.0 / sample_rate as f64;
    let target_len = ((mono_samples_native.len() as f64) * ratio) as usize;
    let mut out_samples = Vec::with_capacity(target_len);

    for i in 0..target_len {
        let src_idx = (i as f64) / ratio;
        let idx0 = src_idx.floor() as usize;
        let frac = (src_idx - idx0 as f64) as f32;
        let s0 = *mono_samples_native.get(idx0).unwrap_or(&0.0);
        let s1 = *mono_samples_native.get(idx0 + 1).unwrap_or(&s0);
        let interp = s0 + frac * (s1 - s0);
        out_samples.push((interp.clamp(-1.0, 1.0) * 32767.0) as i16);
    }

    Ok(out_samples)
}

/// Synthesizes a modern, crisp marimba chime ringtone (3.0s total: 1.4s melody + 1.6s silence).
fn synth_marimba() -> Vec<i16> {
    let total_samples = (3.0 * SAMPLE_RATE as f32) as usize; // 24,000 samples
    let mut samples = vec![0i16; total_samples];

    // Note events: (start_time_secs, frequency_hz)
    let notes = [
        (0.00, 659.25),  // E5
        (0.16, 830.61),  // G#5
        (0.32, 987.77),  // B5
        (0.48, 1318.51), // E6
        (0.80, 830.61),  // G#5
        (0.96, 987.77),  // B5
        (1.12, 1318.51), // E6
    ];

    for (start_t, freq) in notes {
        let start_sample = (start_t * SAMPLE_RATE as f32) as usize;
        let note_samples = (0.65 * SAMPLE_RATE as f32) as usize;

        for i in 0..note_samples {
            let target_idx = start_sample + i;
            if target_idx >= total_samples {
                break;
            }
            let dt = i as f32 / SAMPLE_RATE as f32;

            // Attack (8ms) and Exponential decay
            let attack = (dt / 0.008).min(1.0);
            let decay = (-5.5 * dt).exp();
            let env = attack * decay;

            // Harmonic wooden bar timbre: fundamental + 2nd harmonic + 3rd harmonic
            let wave = 0.65 * (2.0 * PI * freq * dt).sin()
                + 0.25 * (2.0 * PI * (2.0 * freq) * dt).sin()
                + 0.10 * (2.0 * PI * (3.0 * freq) * dt).sin();

            let sample_val = (wave * env * 24000.0) as i32;
            let current = samples[target_idx] as i32;
            samples[target_idx] =
                (current + sample_val).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    samples
}

/// Synthesizes a classic dual-tone telephone bell ring (1.0s ring, 2.0s silence).
fn synth_classic_bell() -> Vec<i16> {
    let total_samples = (3.0 * SAMPLE_RATE as f32) as usize;
    let ring_samples = (1.0 * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0i16; total_samples];

    let f1 = 753.0f32;
    let f2 = 840.0f32;
    let f_overtone = 1593.0f32;

    for i in 0..ring_samples {
        let t = i as f32 / SAMPLE_RATE as f32;

        // 20 Hz warble modulation
        let warble = 0.65 + 0.35 * (2.0 * PI * 20.0 * t).sin();

        // Attack (15ms) and release (35ms) envelope
        let attack = (t / 0.015).min(1.0);
        let release = ((1.0 - t) / 0.035).min(1.0).max(0.0);
        let env = attack * release;

        let wave = 0.45 * (2.0 * PI * f1 * t).sin()
            + 0.45 * (2.0 * PI * f2 * t).sin()
            + 0.10 * (2.0 * PI * f_overtone * t).sin();

        let val = (wave * warble * env * 20000.0) as i16;
        samples[i] = val;
    }

    samples
}

/// Synthesizes an acoustic, gentle melodic chime progression (warm and relaxing).
fn synth_acoustic() -> Vec<i16> {
    let total_samples = (3.0 * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0i16; total_samples];

    let notes = [
        (0.00, 523.25),  // C5
        (0.28, 659.25),  // E5
        (0.56, 783.99),  // G5
        (0.84, 1046.50), // C6
    ];

    for (start_t, freq) in notes {
        let start_sample = (start_t * SAMPLE_RATE as f32) as usize;
        let note_samples = (0.90 * SAMPLE_RATE as f32) as usize;

        for i in 0..note_samples {
            let target_idx = start_sample + i;
            if target_idx >= total_samples {
                break;
            }
            let dt = i as f32 / SAMPLE_RATE as f32;

            let attack = (dt / 0.025).min(1.0);
            let decay = (-3.0 * dt).exp();
            let env = attack * decay;

            let wave =
                0.70 * (2.0 * PI * freq * dt).sin() + 0.30 * (2.0 * PI * (2.0 * freq) * dt).sin();

            let sample_val = (wave * env * 22000.0) as i32;
            let current = samples[target_idx] as i32;
            samples[target_idx] =
                (current + sample_val).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    samples
}

/// Synthesizes the standard European 425 Hz PBX ringtone with smoothed edge transitions.
fn synth_standard_tone() -> Vec<i16> {
    let total_samples = (3.0 * SAMPLE_RATE as f32) as usize;
    let tone_samples = (1.0 * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0i16; total_samples];

    for i in 0..tone_samples {
        let t = i as f32 / SAMPLE_RATE as f32;

        let attack = (t / 0.020).min(1.0);
        let release = ((1.0 - t) / 0.020).min(1.0).max(0.0);
        let env = attack * release;

        let sample = (32767.0 * 0.4 * env * (2.0 * PI * 425.0 * t).sin()) as i16;
        samples[i] = sample;
    }

    samples
}
