//! RTP Media Session management for audio streaming, jitter buffering, and DTMF telephony events.

use crate::dtmf::DtmfEvent;
use crate::jitter_buffer::JitterBuffer;
use crate::rtp::RtpPacket;
use crate::stats::MediaStats;
use bytes::Bytes;
use sip_audio::g711::{alaw_to_linear, linear_to_alaw, linear_to_ulaw, ulaw_to_linear};
use sip_domain::AudioCodec;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

#[derive(Error, Debug)]
pub enum MediaError {
    #[error("Socket I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Channel closed")]
    ChannelClosed,
    #[error("Unsupported codec: {0:?}")]
    UnsupportedCodec(AudioCodec),
}

#[derive(Debug, Clone)]
pub struct MediaSessionConfig {
    pub local_bind_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: AudioCodec,
    pub dtmf_payload_type: Option<u8>,
    pub ssrc: u32,
    pub target_jitter_depth_packets: usize,
    pub max_jitter_capacity_packets: usize,
}

impl Default for MediaSessionConfig {
    fn default() -> Self {
        Self {
            local_bind_addr: "0.0.0.0:0".parse().unwrap(),
            remote_addr: "127.0.0.1:10000".parse().unwrap(),
            codec: AudioCodec::Pcma,
            dtmf_payload_type: Some(101),
            ssrc: 0x12345678,
            target_jitter_depth_packets: 3,
            max_jitter_capacity_packets: 20,
        }
    }
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || octets[0] == 127
                || (octets[0] == 169 && octets[1] == 254)
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

pub struct MediaSession {
    stats: Arc<RwLock<MediaStats>>,
    is_muted: Arc<AtomicBool>,
    pcm_tx: mpsc::Sender<Vec<i16>>,
    dtmf_tx: mpsc::Sender<char>,
    cancel_token: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
    local_addr: SocketAddr,
    destination_addr: Arc<RwLock<SocketAddr>>,
    latched: Arc<AtomicBool>,
}

impl MediaSession {
    /// Spawns a new active RTP media session.
    ///
    /// Returns:
    /// - `MediaSession` handle for control, muting, DTMF sending, stats, and teardown.
    /// - `mpsc::Receiver<Vec<i16>>` providing received 160-sample 8 kHz PCM audio frames for playback.
    /// - `mpsc::Receiver<DtmfEvent>` delivering incoming RFC 4733 DTMF events.
    pub async fn start(
        config: MediaSessionConfig,
    ) -> Result<(Self, mpsc::Receiver<Vec<i16>>, mpsc::Receiver<DtmfEvent>), MediaError> {
        // Enforce remote address and port validation (reject 0.0.0.0, multicast, or port 0)
        if config.remote_addr.port() == 0 {
            return Err(MediaError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Remote media port cannot be 0",
            )));
        }
        if config.remote_addr.ip().is_unspecified() || config.remote_addr.ip().is_multicast() {
            return Err(MediaError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Remote media IP cannot be unspecified (0.0.0.0) or multicast",
            )));
        }

        let socket = Arc::new(UdpSocket::bind(config.local_bind_addr).await?);
        let local_addr = socket.local_addr()?;

        let stats = Arc::new(RwLock::new(MediaStats {
            negotiated_codec: format!("{:?}", config.codec),
            ..Default::default()
        }));

        let is_muted = Arc::new(AtomicBool::new(false));
        let cancel_token = CancellationToken::new();

        // Audio transmit channel (from mic/generator into RTP sender)
        let (pcm_tx, mut pcm_in_rx) = mpsc::channel::<Vec<i16>>(64);
        // Audio receive channel (from RTP receiver/jitter buffer to playback)
        let (pcm_out_tx, pcm_out_rx) = mpsc::channel::<Vec<i16>>(64);
        // DTMF outgoing send queue
        let (dtmf_tx, mut dtmf_in_rx) = mpsc::channel::<char>(32);
        // DTMF incoming receive channel
        let (dtmf_rx_tx, dtmf_rx_rx) = mpsc::channel::<DtmfEvent>(32);

        let jitter_buffer = Arc::new(Mutex::new(JitterBuffer::new(
            config.target_jitter_depth_packets,
            config.max_jitter_capacity_packets,
        )));

        let codec_payload_type = match config.codec {
            AudioCodec::Pcma => 8u8,
            AudioCodec::Pcmu => 0u8,
            _ => return Err(MediaError::UnsupportedCodec(config.codec)),
        };

        let mut tasks = Vec::new();

        // 1. RTP Playout task (pops from JitterBuffer every 20ms and forwards decoded PCM to pcm_out_tx)
        {
            let jb = Arc::clone(&jitter_buffer);
            let stats = Arc::clone(&stats);
            let token = cancel_token.clone();
            let codec = config.codec;
            let pcm_out_tx = pcm_out_tx.clone();

            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(20));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut last_frame: Option<Vec<i16>> = None;
                let mut concealment_streak: usize = 0;

                loop {
                    tokio::select! {
                        _ = token.cancelled() => break,
                        _ = interval.tick() => {
                            let mut jb_guard = jb.lock().await;
                            let is_prebuffering = jb_guard.is_prebuffering();
                            let pkt = jb_guard.pop();
                            let lost = jb_guard.packets_lost();
                            drop(jb_guard);

                            // Update lost packets in stats
                            {
                                let mut s = stats.write().await;
                                s.packets_lost = lost;
                            }

                            if let Some(pkt) = pkt {
                                let mut linear_samples = Vec::with_capacity(pkt.payload.len());
                                match codec {
                                    AudioCodec::Pcma => {
                                        for &b in pkt.payload.iter() {
                                            linear_samples.push(alaw_to_linear(b));
                                        }
                                    }
                                    AudioCodec::Pcmu => {
                                        for &b in pkt.payload.iter() {
                                            linear_samples.push(ulaw_to_linear(b));
                                        }
                                    }
                                    _ => {}
                                }
                                last_frame = Some(linear_samples.clone());
                                concealment_streak = 0;
                                match pcm_out_tx.try_send(linear_samples) {
                                    Ok(()) => {}
                                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                        // Downstream consumer is slow or hasn't started; avoid stalling RTP clock
                                    }
                                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                        break;
                                    }
                                }
                            } else if !is_prebuffering {
                                // Packet loss concealment (PLC)
                                {
                                    let mut s = stats.write().await;
                                    s.plc_frames_concealed += 1;
                                }
                                concealment_streak += 1;
                                let concealed = if let Some(prev) = &last_frame {
                                    let decay = if concealment_streak <= 3 {
                                        0.7f32.powi(concealment_streak as i32)
                                    } else {
                                        0.0
                                    };
                                    prev.iter().map(|&s| (s as f32 * decay) as i16).collect()
                                } else {
                                    vec![0i16; 160]
                                };
                                match pcm_out_tx.try_send(concealed) {
                                    Ok(()) => {}
                                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            });
            tasks.push(handle);
        }

        let destination_addr = Arc::new(RwLock::new(config.remote_addr));
        let latched = Arc::new(AtomicBool::new(false));

        // 2. RTP Receiver task (reads UDP packets, calculates RFC 3550 jitter, pushes to JitterBuffer)
        {
            let socket = Arc::clone(&socket);
            let jb = Arc::clone(&jitter_buffer);
            let stats = Arc::clone(&stats);
            let token = cancel_token.clone();
            let dtmf_payload_type = config.dtmf_payload_type;
            let dest_addr_recv = Arc::clone(&destination_addr);
            let latched_recv = Arc::clone(&latched);

            let handle = tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                let start_instant = tokio::time::Instant::now();
                let mut prev_transit: Option<f64> = None;
                let mut jitter_estimate: f64 = 0.0;

                loop {
                    tokio::select! {
                        _ = token.cancelled() => break,
                        recv_res = socket.recv_from(&mut buf) => {
                            match recv_res {
                                Ok((len, peer)) => {
                                    let current_dest = *dest_addr_recv.read().await;

                                    let packet = match RtpPacket::parse(&buf[..len]) {
                                        Ok(p) => p,
                                        Err(e) => {
                                            warn!(len, "Failed to parse incoming UDP packet as RTP: {:?}", e);
                                            continue;
                                        }
                                    };

                                    let is_expected_pt = packet.payload_type == codec_payload_type
                                        || dtmf_payload_type == Some(packet.payload_type);

                                    if !is_expected_pt {
                                        warn!(
                                            pt = packet.payload_type,
                                            expected_codec = codec_payload_type,
                                            dtmf_pt = ?dtmf_payload_type,
                                            "Dropping packet with unnegotiated payload type"
                                        );
                                        continue;
                                    }

                                    // RFC 4961 Symmetric RTP Latching with SEC-04 Validation
                                    if peer != current_dest {
                                        if !latched_recv.load(Ordering::Relaxed) {
                                            let orig_ip = config.remote_addr.ip();
                                            let can_latch = peer.ip() == orig_ip
                                                || (orig_ip.is_loopback() && peer.ip().is_loopback())
                                                || orig_ip.is_unspecified()
                                                || is_private_ip(orig_ip)
                                                || is_private_ip(peer.ip());

                                            if can_latch {
                                                *dest_addr_recv.write().await = peer;
                                                latched_recv.store(true, Ordering::Relaxed);
                                                tracing::info!(
                                                    "Symmetric RTP: Latched destination address to peer"
                                                );
                                            } else {
                                                warn!(
                                                    "Symmetric RTP: Rejected latching from unauthorized public IP"
                                                );
                                                continue;
                                            }
                                        } else if peer.ip() != current_dest.ip() && !peer.ip().is_loopback() && !current_dest.ip().is_loopback() {
                                            warn!("Dropping unauthorized RTP packet from unexpected source IP");
                                            continue;
                                        }
                                    }

                                    let now_secs = (tokio::time::Instant::now() - start_instant).as_secs_f64();
                                    let arrival_ts = now_secs * 8000.0;

                                    // Update stats
                                    let pkts_received = {
                                        let mut s = stats.write().await;
                                        s.packets_received += 1;
                                        s.bytes_received += len as u64;

                                        // RFC 3550 jitter calculation
                                        let packet_ts = packet.timestamp as f64;
                                        let transit = arrival_ts - packet_ts;
                                        if let Some(prev) = prev_transit {
                                            let diff = (transit - prev).abs();
                                            jitter_estimate += (diff - jitter_estimate) / 16.0;
                                            // Convert 8 kHz timestamp units to milliseconds
                                            s.jitter_estimate_ms = (jitter_estimate / 8.0) as f32;
                                        }
                                        prev_transit = Some(transit);
                                        s.packets_received
                                    };

                                    if pkts_received == 1 || pkts_received % 250 == 0 {
                                        tracing::info!(
                                            pkts_received,
                                            peer = %peer,
                                            pt = packet.payload_type,
                                            seq = packet.sequence_number,
                                            "RTP packet received from peer"
                                        );
                                    }

                                    // Handle DTMF or Audio
                                    if let Some(dtmf_pt) = dtmf_payload_type {
                                        if packet.payload_type == dtmf_pt {
                                            if let Ok(event) = DtmfEvent::parse(&packet.payload) {
                                                let _ = dtmf_rx_tx.send(event).await;
                                            }
                                            continue;
                                        }
                                    }

                                    if packet.payload_type == codec_payload_type {
                                        let mut jb_guard = jb.lock().await;
                                        jb_guard.push(packet);
                                    }
                                }
                                Err(e) => {
                                    if !token.is_cancelled() {
                                        warn!("RTP socket recv error: {}", e);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            });
            tasks.push(handle);
        }

        // 3. RTP Sender task (encodes PCM frames at 20ms intervals and sends RTP packets)
        {
            let socket = Arc::clone(&socket);
            let stats = Arc::clone(&stats);
            let is_muted = Arc::clone(&is_muted);
            let token = cancel_token.clone();
            let dest_addr_send = Arc::clone(&destination_addr);
            let codec = config.codec;
            let ssrc = config.ssrc;
            let dtmf_payload_type = config.dtmf_payload_type;

            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(20));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                let mut seq: u16 = 1000;
                let mut timestamp: u32 = 160000;
                let mut active_dtmf: Option<(char, u16, u8)> = None; // (char, duration, end_count)

                loop {
                    tokio::select! {
                        _ = token.cancelled() => break,
                        _ = interval.tick() => {
                            let target = *dest_addr_send.read().await;

                            // Check if a new DTMF digit should be triggered
                            if active_dtmf.is_none() {
                                if let Ok(digit) = dtmf_in_rx.try_recv() {
                                    active_dtmf = Some((digit, 160, 0));
                                }
                            }

                            // 1. Send DTMF if active
                            if let Some((digit, ref mut dur, ref mut end_count)) = active_dtmf {
                                if let Some(dtmf_pt) = dtmf_payload_type {
                                    let is_end = *dur >= 800; // 100ms tone duration = 800 units
                                    if let Ok(event) = DtmfEvent::from_char(digit, is_end, *dur) {
                                        let raw_event = event.serialize();
                                        let pkt = RtpPacket::new(dtmf_pt, seq, timestamp, ssrc, raw_event);
                                        let bytes = pkt.serialize();
                                        let _ = socket.send_to(&bytes, target).await;

                                        seq = seq.wrapping_add(1);

                                        let mut s = stats.write().await;
                                        s.packets_sent += 1;
                                        s.bytes_sent += bytes.len() as u64;
                                    }

                                    if is_end {
                                        *end_count += 1;
                                        if *end_count >= 3 {
                                            active_dtmf = None;
                                            timestamp = timestamp.wrapping_add(160);
                                        }
                                    } else {
                                        *dur += 160;
                                    }
                                    continue;
                                }
                            }

                            // 2. Otherwise send normal 20ms audio frame
                            // Read 160 PCM samples from queue or use silence
                            let samples = match pcm_in_rx.try_recv() {
                                Ok(s) if s.len() == 160 => s,
                                _ => vec![0i16; 160], // Silence
                            };

                            let muted = is_muted.load(Ordering::Relaxed);
                            let mut payload = Vec::with_capacity(160);

                            for &s in &samples {
                                let sample = if muted { 0 } else { s };
                                match codec {
                                    AudioCodec::Pcma => payload.push(linear_to_alaw(sample)),
                                    AudioCodec::Pcmu => payload.push(linear_to_ulaw(sample)),
                                    _ => {}
                                }
                            }

                            let pkt = RtpPacket::new(codec_payload_type, seq, timestamp, ssrc, Bytes::from(payload));
                            let serialized = pkt.serialize();

                            if let Ok(bytes_sent) = socket.send_to(&serialized, target).await {
                                let pkts_sent = {
                                    let mut s = stats.write().await;
                                    s.packets_sent += 1;
                                    s.bytes_sent += bytes_sent as u64;
                                    s.packets_sent
                                };
                                if pkts_sent == 1 || pkts_sent % 250 == 0 {
                                    tracing::info!(
                                        pkts_sent,
                                        codec = ?codec,
                                        "RTP packet sent to remote"
                                    );
                                }
                            }

                            seq = seq.wrapping_add(1);
                            timestamp = timestamp.wrapping_add(160);
                        }
                    }
                }
            });
            tasks.push(handle);
        }

        Ok((
            Self {
                stats,
                is_muted,
                pcm_tx,
                dtmf_tx,
                cancel_token,
                tasks,
                local_addr,
                destination_addr,
                latched,
            },
            pcm_out_rx,
            dtmf_rx_rx,
        ))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn destination_addr(&self) -> SocketAddr {
        *self.destination_addr.read().await
    }

    pub async fn update_destination(&self, new_dest: SocketAddr) {
        *self.destination_addr.write().await = new_dest;
        self.latched.store(false, Ordering::Relaxed);
    }

    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::SeqCst);
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::SeqCst)
    }

    pub async fn send_audio_frame(&self, frame: Vec<i16>) -> Result<(), MediaError> {
        self.pcm_tx
            .send(frame)
            .await
            .map_err(|_| MediaError::ChannelClosed)
    }

    pub async fn send_dtmf(&self, digit: char) -> Result<(), MediaError> {
        self.dtmf_tx
            .send(digit)
            .await
            .map_err(|_| MediaError::ChannelClosed)
    }

    pub async fn get_stats(&self) -> MediaStats {
        self.stats.read().await.clone()
    }

    pub async fn stop(mut self) -> MediaStats {
        self.cancel_token.cancel();
        for task in self.tasks.drain(..) {
            let _ = tokio::time::timeout(Duration::from_millis(500), task).await;
        }
        let stats = self.get_stats().await;
        tracing::info!(
            packets_sent = stats.packets_sent,
            bytes_sent = stats.bytes_sent,
            packets_received = stats.packets_received,
            bytes_received = stats.bytes_received,
            packets_lost = stats.packets_lost,
            plc_frames = stats.plc_frames_concealed,
            jitter_ms = stats.jitter_estimate_ms,
            "RTP MediaSession finished"
        );
        stats
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}
