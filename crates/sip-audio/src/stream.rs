use crate::device::{find_input_device, find_output_device};
use crate::error::AudioError;
use crate::resampler::{InputResampler, OutputResampler};
use cpal::Stream;
use cpal::traits::{DeviceTrait, StreamTrait};
use rtrb::RingBuffer;
use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBackendMode {
    RealHardware,
    Synthetic,
}

pub struct AudioEngine {
    input_stream: Option<Stream>,
    output_stream: Option<Stream>,
    input_pump: Option<JoinHandle<()>>,
    output_pump: Option<JoinHandle<()>>,
    is_muted: Arc<AtomicBool>,
    mode: AudioBackendMode,
    samples_captured: Arc<AtomicU64>,
    samples_played: Arc<AtomicU64>,
}

impl AudioEngine {
    /// Creates an AudioEngine in RealHardware mode (will attempt to open real CPAL sound devices).
    pub fn new() -> Self {
        Self {
            input_stream: None,
            output_stream: None,
            input_pump: None,
            output_pump: None,
            is_muted: Arc::new(AtomicBool::new(false)),
            mode: AudioBackendMode::RealHardware,
            samples_captured: Arc::new(AtomicU64::new(0)),
            samples_played: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Creates an AudioEngine in Synthetic mode (generates/consumes synthetic PCM frames without hardware devices).
    pub fn new_synthetic() -> Self {
        Self {
            input_stream: None,
            output_stream: None,
            input_pump: None,
            output_pump: None,
            is_muted: Arc::new(AtomicBool::new(false)),
            mode: AudioBackendMode::Synthetic,
            samples_captured: Arc::new(AtomicU64::new(0)),
            samples_played: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn mode(&self) -> AudioBackendMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: AudioBackendMode) {
        self.mode = mode;
    }

    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::SeqCst);
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::SeqCst)
    }

    pub fn samples_captured(&self) -> u64 {
        self.samples_captured.load(Ordering::Relaxed)
    }

    pub fn samples_played(&self) -> u64 {
        self.samples_played.load(Ordering::Relaxed)
    }

    pub fn is_input_active(&self) -> bool {
        self.input_stream.is_some() || self.input_pump.is_some()
    }

    pub fn is_output_active(&self) -> bool {
        self.output_stream.is_some() || self.output_pump.is_some()
    }

    /// Starts microphone capture. Emits 160-sample (20ms at 8kHz) mono PCM chunks to `tx`.
    pub fn start_input(
        &mut self,
        device_id: Option<&str>,
        tx: mpsc::Sender<Vec<i16>>,
    ) -> Result<(), AudioError> {
        self.stop_input();

        if self.mode == AudioBackendMode::Synthetic {
            return self.start_synthetic_input(tx);
        }

        // Try opening real hardware device
        let device = match find_input_device(device_id) {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "No real input audio device found ({}), falling back to synthetic capture",
                    e
                );
                return self.start_synthetic_input(tx);
            }
        };

        let default_config = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    "Failed to query default input config ({}), falling back to synthetic capture",
                    e
                );
                return self.start_synthetic_input(tx);
            }
        };

        let sample_rate = default_config.sample_rate();
        let channels = default_config.channels();
        let sample_format = default_config.sample_format();
        let mut stream_config: cpal::StreamConfig = default_config.clone().into();
        match default_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } if *min <= 1024 && 1024 <= *max => {
                stream_config.buffer_size = cpal::BufferSize::Fixed(1024);
            }
            _ => {}
        }
        info!(
            device = %device.description().map(|d| d.name().to_string()).unwrap_or_default(),
            sample_rate = sample_rate,
            channels = channels,
            format = ?sample_format,
            buffer_size = ?stream_config.buffer_size,
            "Starting CPAL hardware input capture"
        );

        // 8192 samples = ~1 second buffer at 8kHz
        let (mut producer, mut consumer) = RingBuffer::<i16>::new(8192);
        let mut resampler = InputResampler::new(sample_rate, channels);
        let is_muted = Arc::clone(&self.is_muted);

        let stream_res = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                stream_config,
                move |data: &[f32], _| {
                    resampler.process_f32(data, &mut producer, is_muted.load(Ordering::Relaxed));
                },
                move |err| {
                    warn!("CPAL input stream error: {}", err);
                },
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                stream_config,
                move |data: &[i16], _| {
                    resampler.process_i16(data, &mut producer, is_muted.load(Ordering::Relaxed));
                },
                move |err| {
                    warn!("CPAL input stream error: {}", err);
                },
                None,
            ),
            format => {
                warn!(
                    "Unsupported input sample format {:?}, falling back to synthetic capture",
                    format
                );
                return self.start_synthetic_input(tx);
            }
        };

        let stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "Failed to build CPAL input stream ({}), falling back to synthetic capture",
                    e
                );
                return self.start_synthetic_input(tx);
            }
        };

        if let Err(e) = stream.play() {
            warn!(
                "Failed to start CPAL input stream ({}), falling back to synthetic capture",
                e
            );
            return self.start_synthetic_input(tx);
        }

        self.input_stream = Some(stream);

        // Async pump: drains rtrb consumer and delivers 160-sample (20ms) frames to tx
        let samples_captured = Arc::clone(&self.samples_captured);
        let pump_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut frame = Vec::with_capacity(160);
            let mut frames_captured_count: u64 = 0;

            loop {
                interval.tick().await;

                while let Ok(sample) = consumer.pop() {
                    frame.push(sample);
                    if frame.len() == 160 {
                        let chunk = std::mem::replace(&mut frame, Vec::with_capacity(160));
                        samples_captured.fetch_add(160, Ordering::Relaxed);
                        frames_captured_count += 1;
                        if frames_captured_count == 1 || frames_captured_count % 250 == 0 {
                            info!(
                                frames_captured_count,
                                "Hardware input capture delivering frames to engine"
                            );
                        }
                        if tx.send(chunk).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        self.input_pump = Some(pump_handle);
        info!("AudioEngine: Real hardware input capture started successfully");
        Ok(())
    }

    fn start_synthetic_input(&mut self, tx: mpsc::Sender<Vec<i16>>) -> Result<(), AudioError> {
        let is_muted = Arc::clone(&self.is_muted);
        let samples_captured = Arc::clone(&self.samples_captured);

        let pump_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(20));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut tick: u32 = 0;

            loop {
                interval.tick().await;
                let muted = is_muted.load(Ordering::Relaxed);

                let mut frame = Vec::with_capacity(160);
                for i in 0..160 {
                    if muted {
                        frame.push(0);
                    } else {
                        // 425 Hz sine tone at 8000 Hz
                        let t = (tick * 160 + i) as f32 / 8000.0;
                        let sample = (32767.0 * 0.3 * (2.0 * PI * 425.0 * t).sin()) as i16;
                        frame.push(sample);
                    }
                }
                tick = tick.wrapping_add(1);
                samples_captured.fetch_add(160, Ordering::Relaxed);

                if tx.send(frame).await.is_err() {
                    break;
                }
            }
        });

        self.input_pump = Some(pump_handle);
        info!("AudioEngine: Synthetic audio input generator started");
        Ok(())
    }

    /// Starts speaker playback. Consumes 160-sample (20ms at 8kHz) mono PCM chunks from `rx`.
    pub fn start_output(
        &mut self,
        device_id: Option<&str>,
        mut rx: mpsc::Receiver<Vec<i16>>,
    ) -> Result<(), AudioError> {
        self.stop_output();

        if self.mode == AudioBackendMode::Synthetic {
            return self.start_synthetic_output(rx);
        }

        // Try opening real hardware device
        let device = match find_output_device(device_id) {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "No real output audio device found ({}), falling back to synthetic playback",
                    e
                );
                return self.start_synthetic_output(rx);
            }
        };

        let default_config = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    "Failed to query default output config ({}), falling back to synthetic playback",
                    e
                );
                return self.start_synthetic_output(rx);
            }
        };

        let sample_rate = default_config.sample_rate();
        let channels = default_config.channels();
        let sample_format = default_config.sample_format();
        let mut stream_config: cpal::StreamConfig = default_config.clone().into();
        match default_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } if *min <= 1024 && 1024 <= *max => {
                stream_config.buffer_size = cpal::BufferSize::Fixed(1024);
            }
            _ => {}
        }
        info!(
            device = %device.description().map(|d| d.name().to_string()).unwrap_or_default(),
            sample_rate = sample_rate,
            channels = channels,
            format = ?sample_format,
            buffer_size = ?stream_config.buffer_size,
            "Starting CPAL hardware output playback"
        );

        let (mut producer, mut consumer) = RingBuffer::<i16>::new(8192);
        let mut resampler = OutputResampler::new(sample_rate, channels);

        let stream_res = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                stream_config,
                move |data: &mut [f32], _| {
                    resampler.fill_f32(data, &mut consumer);
                },
                move |err| {
                    warn!("CPAL output stream error: {}", err);
                },
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                stream_config,
                move |data: &mut [i16], _| {
                    resampler.fill_i16(data, &mut consumer);
                },
                move |err| {
                    warn!("CPAL output stream error: {}", err);
                },
                None,
            ),
            format => {
                warn!(
                    "Unsupported output sample format {:?}, falling back to synthetic playback",
                    format
                );
                return self.start_synthetic_output(rx);
            }
        };

        let stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "Failed to build CPAL output stream ({}), falling back to synthetic playback",
                    e
                );
                return self.start_synthetic_output(rx);
            }
        };

        if let Err(e) = stream.play() {
            warn!(
                "Failed to start CPAL output stream ({}), falling back to synthetic playback",
                e
            );
            return self.start_synthetic_output(rx);
        }

        self.output_stream = Some(stream);

        // Async pump: reads 160-sample chunks from rx and feeds rtrb producer
        let samples_played = Arc::clone(&self.samples_played);
        let pump_handle = tokio::spawn(async move {
            let mut frames_played_count: u64 = 0;
            while let Some(frame) = rx.recv().await {
                frames_played_count += 1;
                if frames_played_count == 1 || frames_played_count % 250 == 0 {
                    info!(
                        frames_played_count,
                        "Hardware output playback receiving frames from engine"
                    );
                }
                let count = frame.len() as u64;
                for sample in frame {
                    let _ = producer.push(sample);
                }
                samples_played.fetch_add(count, Ordering::Relaxed);
            }
        });

        self.output_pump = Some(pump_handle);
        info!("AudioEngine: Real hardware speaker playback started successfully");
        Ok(())
    }

    fn start_synthetic_output(
        &mut self,
        mut rx: mpsc::Receiver<Vec<i16>>,
    ) -> Result<(), AudioError> {
        let samples_played = Arc::clone(&self.samples_played);

        let pump_handle = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                samples_played.fetch_add(frame.len() as u64, Ordering::Relaxed);
            }
        });

        self.output_pump = Some(pump_handle);
        info!("AudioEngine: Synthetic audio playback receiver started");
        Ok(())
    }

    pub fn stop_input(&mut self) {
        if let Some(handle) = self.input_pump.take() {
            handle.abort();
        }
        self.input_stream = None;
    }

    pub fn stop_output(&mut self) {
        if let Some(handle) = self.output_pump.take() {
            handle.abort();
        }
        self.output_stream = None;
    }

    pub fn stop(&mut self) {
        self.stop_input();
        self.stop_output();
    }

    /// Generates standard European / German dial / ringing tone samples (425 Hz sine wave at 8 kHz PCM).
    pub fn generate_call_progress_tone(
        freq_hz: f32,
        duration_ms: u32,
        sample_rate: u32,
    ) -> Vec<i16> {
        let total_samples = (duration_ms as usize * sample_rate as usize) / 1000;
        let mut samples = Vec::with_capacity(total_samples);
        for i in 0..total_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (32767.0 * 0.4 * (2.0 * PI * freq_hz * t).sin()) as i16;
            samples.push(sample);
        }
        samples
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
