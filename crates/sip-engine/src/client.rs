//! High-level SIP Client handling network loop and message dispatch.

use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, trace};

use sip_protocol::{SipMessage, SipRequest, TransactionManager};
use sip_transport::SipUdpTransport;

#[async_trait]
pub trait IncomingRequestHandler: Send + Sync {
    async fn handle_request(&self, req: SipRequest, src: SocketAddr) -> bool;
}

pub struct SipClient {
    transport: Arc<SipUdpTransport>,
    transactions: Arc<TransactionManager>,
    request_handler: Option<Arc<dyn IncomingRequestHandler>>,
}

impl SipClient {
    pub fn new(transport: Arc<SipUdpTransport>, transactions: Arc<TransactionManager>) -> Self {
        Self {
            transport,
            transactions,
            request_handler: None,
        }
    }

    pub fn with_request_handler(
        mut self,
        request_handler: Arc<dyn IncomingRequestHandler>,
    ) -> Self {
        self.request_handler = Some(request_handler);
        self
    }

    /// Spawns the background receive loop processing incoming SIP UDP datagrams
    pub fn start_receive_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut buf = [0u8; 65535];
            loop {
                match self.transport.recv_from(&mut buf).await {
                    Ok((len, src)) => {
                        if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                            trace!(bytes = len, "Received incoming SIP UDP datagram");
                            match SipMessage::parse(text) {
                                Ok(SipMessage::Response(res)) => {
                                    if !self.transactions.handle_response(res).await {
                                        debug!("Unhandled SIP response received");
                                    }
                                }
                                Ok(SipMessage::Request(req)) => {
                                    debug!(method = %req.method, "Incoming SIP request received");
                                    if let Some(ref handler) = self.request_handler {
                                        if !handler.handle_request(req.clone(), src).await {
                                            debug!(method = %req.method, "Incoming SIP request not handled");
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!(error = %e, "Failed to parse incoming SIP message");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Fatal socket read error in SIP receive loop");
                        break;
                    }
                }
            }
        })
    }
}
