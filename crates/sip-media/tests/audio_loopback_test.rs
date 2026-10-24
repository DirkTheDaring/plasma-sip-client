use sip_audio::stream::AudioEngine;
use sip_domain::AudioCodec;
use sip_media::{MediaSession, MediaSessionConfig};
use std::time::Duration;

#[tokio::test]
async fn test_synthetic_bidirectional_audio_loopback() {
    // 1. Setup Session A and Session B with temporary dummy remote addresses
    let dummy_addr = "127.0.0.1:9".parse().unwrap();
    let config_a = MediaSessionConfig {
        local_bind_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: dummy_addr,
        codec: AudioCodec::Pcma,
        dtmf_payload_type: Some(101),
        ssrc: 0x11111111,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let config_b = MediaSessionConfig {
        local_bind_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: dummy_addr,
        codec: AudioCodec::Pcma,
        dtmf_payload_type: Some(101),
        ssrc: 0x22222222,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let (session_a, _pcm_rx_a, _dtmf_rx_a) =
        MediaSession::start(config_a).await.expect("session A");
    let (session_b, _pcm_rx_b, _dtmf_rx_b) =
        MediaSession::start(config_b).await.expect("session B");

    let addr_a = session_a.local_addr();
    let addr_b = session_b.local_addr();

    // Now re-create sessions with exact peer endpoints
    let _ = session_a.stop().await;
    let _ = session_b.stop().await;

    let config_a = MediaSessionConfig {
        local_bind_addr: addr_a,
        remote_addr: addr_b,
        codec: AudioCodec::Pcma,
        dtmf_payload_type: Some(101),
        ssrc: 0x11111111,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let config_b = MediaSessionConfig {
        local_bind_addr: addr_b,
        remote_addr: addr_a,
        codec: AudioCodec::Pcma,
        dtmf_payload_type: Some(101),
        ssrc: 0x22222222,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let (session_a, _pcm_rx_a, _dtmf_rx_a) =
        MediaSession::start(config_a).await.expect("session A peer");
    let (session_b, mut pcm_rx_b, mut dtmf_rx_b) =
        MediaSession::start(config_b).await.expect("session B peer");

    // 2. Transmit synthetic 425 Hz tone from A to B
    let tone_frame = AudioEngine::generate_call_progress_tone(425.0, 20, 8000);
    assert_eq!(tone_frame.len(), 160);

    // Send 10 frames (200 ms of audio)
    for _ in 0..10 {
        session_a
            .send_audio_frame(tone_frame.clone())
            .await
            .expect("send frame");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Receive audio frames on Session B and verify non-zero waveform energy
    let mut received_frames = 0;
    let mut non_zero_samples = 0;

    let timeout = tokio::time::sleep(Duration::from_millis(600));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => break,
            frame = pcm_rx_b.recv() => {
                if let Some(f) = frame {
                    received_frames += 1;
                    for s in f {
                        if s != 0 {
                            non_zero_samples += 1;
                        }
                    }
                    if received_frames >= 5 {
                        break;
                    }
                }
            }
        }
    }

    assert!(
        received_frames >= 3,
        "Expected at least 3 audio frames, got {}",
        received_frames
    );
    assert!(
        non_zero_samples > 100,
        "Expected non-zero audio content in received tone frames"
    );

    // 4. Test DTMF transmission from A to B
    session_a.send_dtmf('9').await.expect("send dtmf 9");

    let dtmf_timeout = tokio::time::sleep(Duration::from_millis(600));
    tokio::pin!(dtmf_timeout);
    let mut received_dtmf = false;

    loop {
        tokio::select! {
            _ = &mut dtmf_timeout => break,
            event = dtmf_rx_b.recv() => {
                if let Some(e) = event {
                    if e.event_id == 9 {
                        received_dtmf = true;
                        break;
                    }
                }
            }
        }
    }

    assert!(
        received_dtmf,
        "Expected to receive RFC 4733 DTMF event for digit '9'"
    );

    // 5. Test Mute functionality
    session_a.set_muted(true);
    assert!(session_a.is_muted());

    // 6. Stop sessions and verify clean teardown and stats
    let stats_a = session_a.stop().await;
    let stats_b = session_b.stop().await;

    assert!(
        stats_a.packets_sent > 0,
        "Session A should have sent packets"
    );
    assert!(
        stats_b.packets_received > 0,
        "Session B should have received packets"
    );
    assert_eq!(stats_a.negotiated_codec, "Pcma");
}

#[tokio::test]
async fn test_pcmu_bidirectional_audio_loopback() {
    let dummy_addr = "127.0.0.1:9".parse().unwrap();
    let config_a = MediaSessionConfig {
        local_bind_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: dummy_addr,
        codec: AudioCodec::Pcmu,
        dtmf_payload_type: Some(101),
        ssrc: 0x33333333,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };
    let config_b = MediaSessionConfig {
        local_bind_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: dummy_addr,
        codec: AudioCodec::Pcmu,
        dtmf_payload_type: Some(101),
        ssrc: 0x44444444,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let (session_a, _, _) = MediaSession::start(config_a).await.unwrap();
    let (session_b, _, _) = MediaSession::start(config_b).await.unwrap();
    let addr_a = session_a.local_addr();
    let addr_b = session_b.local_addr();
    let _ = session_a.stop().await;
    let _ = session_b.stop().await;

    let config_a = MediaSessionConfig {
        local_bind_addr: addr_a,
        remote_addr: addr_b,
        codec: AudioCodec::Pcmu,
        dtmf_payload_type: Some(101),
        ssrc: 0x33333333,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };
    let config_b = MediaSessionConfig {
        local_bind_addr: addr_b,
        remote_addr: addr_a,
        codec: AudioCodec::Pcmu,
        dtmf_payload_type: Some(101),
        ssrc: 0x44444444,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let (session_a, _, _) = MediaSession::start(config_a).await.unwrap();
    let (session_b, mut pcm_rx_b, _) = MediaSession::start(config_b).await.unwrap();

    let tone_frame = AudioEngine::generate_call_progress_tone(425.0, 20, 8000);
    for _ in 0..6 {
        session_a
            .send_audio_frame(tone_frame.clone())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut received = 0;
    for _ in 0..5 {
        if let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(100), pcm_rx_b.recv()).await
        {
            received += 1;
        }
    }
    assert!(
        received >= 2,
        "Expected at least 2 PCMU audio frames, got {}",
        received
    );

    let stats_a = session_a.stop().await;
    let stats_b = session_b.stop().await;
    assert_eq!(stats_a.negotiated_codec, "Pcmu");
    assert_eq!(stats_b.negotiated_codec, "Pcmu");
}

#[tokio::test]
async fn test_packet_loss_concealment_and_stats() {
    use bytes::Bytes;
    use sip_audio::g711::linear_to_alaw;
    use sip_media::rtp::RtpPacket;
    use tokio::net::UdpSocket;

    let target_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = MediaSessionConfig {
        local_bind_addr: target_addr,
        remote_addr: "127.0.0.1:9".parse().unwrap(),
        codec: AudioCodec::Pcma,
        dtmf_payload_type: Some(101),
        ssrc: 0x55555555,
        target_jitter_depth_packets: 2,
        max_jitter_capacity_packets: 10,
    };

    let (session, mut pcm_rx, _) = MediaSession::start(config).await.unwrap();
    let media_addr = session.local_addr();

    // Raw UDP socket to simulate network packet loss
    let raw_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    // Prepare 160-sample tone payload
    let tone = AudioEngine::generate_call_progress_tone(425.0, 20, 8000);
    let alaw_payload: Vec<u8> = tone.iter().map(|&s| linear_to_alaw(s)).collect();

    // Send packet seq 100, 101, SKIP 102 (simulate packet loss!), send 103
    let p100 = RtpPacket::new(8, 100, 0, 0x9999, Bytes::copy_from_slice(&alaw_payload));
    let p101 = RtpPacket::new(8, 101, 160, 0x9999, Bytes::copy_from_slice(&alaw_payload));
    let p103 = RtpPacket::new(8, 103, 480, 0x9999, Bytes::copy_from_slice(&alaw_payload));

    raw_socket
        .send_to(&p100.serialize(), media_addr)
        .await
        .unwrap();
    raw_socket
        .send_to(&p101.serialize(), media_addr)
        .await
        .unwrap();
    // Packet 102 is LOST!
    raw_socket
        .send_to(&p103.serialize(), media_addr)
        .await
        .unwrap();

    // Read playout frames
    let mut frames = Vec::new();
    let timeout = tokio::time::sleep(Duration::from_millis(400));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => break,
            frame = pcm_rx.recv() => {
                if let Some(f) = frame {
                    frames.push(f);
                    if frames.len() >= 4 {
                        break;
                    }
                }
            }
        }
    }

    assert!(
        frames.len() >= 3,
        "Playout should emit frames without stalling, got {}",
        frames.len()
    );

    let stats = session.stop().await;
    println!("Stats after simulated packet loss: {:?}", stats);
    assert!(
        stats.packets_lost >= 1,
        "Should have registered lost packet: {:?}",
        stats
    );
    assert!(
        stats.plc_frames_concealed >= 1,
        "Should have generated PLC concealed frame: {:?}",
        stats
    );
}
