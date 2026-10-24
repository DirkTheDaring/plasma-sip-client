//! Asynchronous network transport for SIP datagrams (UDP).

pub mod dns;
pub use dns::{resolve_sip_srv, resolve_sip_target};

use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::UdpSocket;
use tracing::{debug, info};

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Packet exceeds maximum transmission unit (MTU): {0} bytes")]
    MessageTooLarge(usize),
}

pub struct SipUdpTransport {
    socket: Arc<UdpSocket>,
    local_addr: SocketAddr,
}

impl SipUdpTransport {
    /// Bind to a local address (e.g. "0.0.0.0:0" for an ephemeral port or "0.0.0.0:5060")
    pub async fn bind(bind_addr: &str) -> Result<Self, TransportError> {
        let socket = UdpSocket::bind(bind_addr).await?;
        let local_addr = socket.local_addr()?;
        info!(local_addr = %local_addr, "Bound SIP UDP transport socket");

        Ok(Self {
            socket: Arc::new(socket),
            local_addr,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn send_to(&self, data: &[u8], target: SocketAddr) -> Result<usize, TransportError> {
        // RFC 3261 Section 18.1.1 recommends checking MTU (< 1300 bytes for UDP)
        if data.len() > 1400 {
            debug!(size = data.len(), "SIP message exceeds recommended UDP MTU");
        }
        let sent = self.socket.send_to(data, target).await?;
        Ok(sent)
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), TransportError> {
        let (len, src) = self.socket.recv_from(buf).await?;
        Ok((len, src))
    }

    pub fn clone_socket(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }

    /// Selects an address matching the socket's address family (IPv4 vs IPv6) from resolved addresses.
    pub fn select_compatible_addr<I>(&self, addrs: I) -> Option<SocketAddr>
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        let is_ipv4 = self.local_addr.is_ipv4();
        let list: Vec<SocketAddr> = addrs.into_iter().collect();
        list.iter()
            .copied()
            .find(|a| a.is_ipv4() == is_ipv4)
            .or_else(|| list.into_iter().next())
    }

    /// Returns the routable local IP address for communicating with `target`.
    /// If bound to 0.0.0.0 or ::, queries OS routing towards the target.
    pub fn local_ip_for(&self, target: SocketAddr) -> std::net::IpAddr {
        if !self.local_addr.ip().is_unspecified() {
            return self.local_addr.ip();
        }
        let probe_bind = if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        if let Ok(sock) = std::net::UdpSocket::bind(probe_bind)
            && sock.connect(target).is_ok()
            && let Ok(local) = sock.local_addr()
        {
            return local.ip();
        }
        self.local_addr.ip()
    }
}
