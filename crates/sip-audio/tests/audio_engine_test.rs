use sip_audio::{
    AudioBackendMode, AudioEngine, find_input_device, find_output_device, list_input_devices,
    list_output_devices,
};
use std::time::Duration;
use tokio::sync::mpsc;

#[test]
fn test_device_enumeration() {
    let inputs = list_input_devices().expect("list input devices");
    let outputs = list_output_devices().expect("list output devices");

    println!(
        "Found {} input devices, {} output devices",
        inputs.len(),
        outputs.len()
    );
    for dev in &inputs {
        println!(
            "Input: id='{}', name='{}', is_default={}",
            dev.id, dev.name, dev.is_default
        );
    }
    for dev in &outputs {
        println!(
            "Output: id='{}', name='{}', is_default={}",
            dev.id, dev.name, dev.is_default
        );
    }

    // Lookup non-existent device should fall back to default
    let dev = find_input_device(Some("non_existent_soundcard_xyz"));
    assert!(dev.is_ok());

    let dev_out = find_output_device(Some("non_existent_soundcard_xyz"));
    assert!(dev_out.is_ok());
}

#[tokio::test]
async fn test_diagnose_jabra() {
    use cpal::traits::DeviceTrait;

    if let Ok(dev) = find_input_device(Some("Jabra Evolve2 30 SE, USB Audio")) {
        println!(
            "Found Jabra input dev desc: {:?}",
            dev.description().map(|d| d.name().to_string())
        );
        if let Ok(cfg) = dev.default_input_config() {
            let mut stream_cfg: cpal::StreamConfig = cfg.into();
            stream_cfg.buffer_size = cpal::BufferSize::Fixed(1024);
            let res = dev.build_input_stream(stream_cfg, |_: &[f32], _| {}, |_| {}, None);
            println!("build_input_stream with Fixed(1024): {:?}", res.is_ok());
        }
    }

    if let Ok(dev) = find_output_device(Some("Jabra Evolve2 30 SE, USB Audio")) {
        println!(
            "Found Jabra output dev desc: {:?}",
            dev.description().map(|d| d.name().to_string())
        );
        if let Ok(cfg) = dev.default_output_config() {
            let mut stream_cfg: cpal::StreamConfig = cfg.into();
            stream_cfg.buffer_size = cpal::BufferSize::Fixed(1024);
            let res = dev.build_output_stream(stream_cfg, |_: &mut [f32], _| {}, |_| {}, None);
            println!("build_output_stream with Fixed(1024): {:?}", res.is_ok());
        }
    }
}

#[tokio::test]
async fn test_jabra_hardware_capture_and_playback() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut engine = AudioEngine::new();
    let (in_tx, mut in_rx) = mpsc::channel::<Vec<i16>>(16);
    let (out_tx, out_rx) = mpsc::channel::<Vec<i16>>(16);

    let dev_name = "Jabra Evolve2 30 SE, USB Audio";
    println!("Testing start_input on Jabra...");
    let in_res = engine.start_input(Some(dev_name), in_tx);
    println!("start_input result: {:?}", in_res);

    println!("Testing start_output on Jabra...");
    let out_res = engine.start_output(Some(dev_name), out_rx);
    println!("start_output result: {:?}", out_res);

    // Send a 425Hz tone to output for 500ms
    tokio::spawn(async move {
        for tick in 0..25 {
            let mut frame = Vec::with_capacity(160);
            for i in 0..160 {
                let t = (tick * 160 + i) as f32 / 8000.0;
                let sample =
                    (32767.0 * 0.3 * (2.0 * std::f32::consts::PI * 425.0 * t).sin()) as i16;
                frame.push(sample);
            }
            if out_tx.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let mut in_count = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(600);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), in_rx.recv()).await {
            Ok(Some(frame)) => {
                in_count += 1;
                let max_sample = frame.iter().map(|&s| s.abs()).max().unwrap_or(0);
                println!("Got input frame #{}, max sample: {}", in_count, max_sample);
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    println!("Total input frames captured: {}", in_count);
    engine.stop();
}

#[tokio::test]
async fn test_synthetic_audio_engine_loopback() {
    let mut engine = AudioEngine::new_synthetic();
    assert_eq!(engine.mode(), AudioBackendMode::Synthetic);

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<i16>>(16);
    let (out_tx, out_rx) = mpsc::channel::<Vec<i16>>(16);

    // Start synthetic input
    engine
        .start_input(None, in_tx)
        .expect("start synthetic input");
    assert!(engine.is_input_active());

    // Start synthetic output
    engine
        .start_output(None, out_rx)
        .expect("start synthetic output");
    assert!(engine.is_output_active());

    // Wait for at least 3 frames (60ms) of synthetic input
    let mut frames_received = 0;
    while frames_received < 3 {
        if let Some(frame) = tokio::time::timeout(Duration::from_millis(200), in_rx.recv())
            .await
            .expect("timeout waiting for frame")
        {
            assert_eq!(frame.len(), 160);
            // Verify non-zero amplitude in synthetic tone
            let non_zero = frame.iter().any(|&s| s != 0);
            assert!(non_zero, "Frame should contain non-zero sine samples");

            // Feed into output
            out_tx.send(frame).await.expect("send to output");
            frames_received += 1;
        }
    }

    // Test mute
    engine.set_muted(true);
    assert!(engine.is_muted());
    // Next frame should be silence
    let muted_frame = tokio::time::timeout(Duration::from_millis(200), in_rx.recv())
        .await
        .expect("timeout")
        .expect("frame");
    assert!(
        muted_frame.iter().all(|&s| s == 0),
        "Muted frame should be silence"
    );

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(engine.samples_captured() >= 480);
    assert!(engine.samples_played() >= 480);

    engine.stop();
    assert!(!engine.is_input_active());
    assert!(!engine.is_output_active());
}

#[tokio::test]
async fn test_real_hardware_audio_engine() {
    let mut engine = AudioEngine::new();
    assert_eq!(engine.mode(), AudioBackendMode::RealHardware);

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<i16>>(16);
    let (out_tx, out_rx) = mpsc::channel::<Vec<i16>>(16);

    // Attempt real hardware start (gracefully falls back if no device or permission)
    let in_res = engine.start_input(None, in_tx);
    assert!(
        in_res.is_ok(),
        "Input start should succeed (or fall back gracefully)"
    );

    let out_res = engine.start_output(None, out_rx);
    assert!(
        out_res.is_ok(),
        "Output start should succeed (or fall back gracefully)"
    );

    // Feed a frame into output
    let test_frame = vec![1000i16; 160];
    let _ = out_tx.send(test_frame).await;

    // Check if input produces a frame within 300ms
    if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_millis(300), in_rx.recv()).await {
        assert_eq!(frame.len(), 160);
    }

    engine.stop();
    assert!(!engine.is_input_active());
    assert!(!engine.is_output_active());
}

#[test]
fn test_ringtone_generation() {
    use sip_audio::ringtone::{SAMPLES_PER_FRAME, generate_ringtone_frames};

    for ringtone in &[
        "marimba",
        "classic_bell",
        "acoustic",
        "standard_tone",
        "ocean",
        "custom",
    ] {
        let frames = generate_ringtone_frames(ringtone, None, 80);
        assert!(
            !frames.is_empty(),
            "Ringtone {} produced empty frames",
            ringtone
        );
        for frame in &frames {
            assert_eq!(frame.len(), SAMPLES_PER_FRAME);
        }
    }

    // Test volume scaling
    let silent_frames = generate_ringtone_frames("marimba", None, 0);
    assert!(silent_frames.iter().all(|f| f.iter().all(|&s| s == 0)));
}
