use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, trace};
use zeroize::Zeroizing;

use sip_auth::{DigestAuthorizer, DigestChallenge};
use sip_domain::{RegistrationState, SipAccount};
use sip_protocol::{
    SipMethod, SipRequest, SipResponse, TransactionKey, TransactionManager, generate_branch,
    generate_tag,
};
use sip_transport::SipUdpTransport;

pub struct RegistrationManager {
    account: SipAccount,
    password: Zeroizing<String>,
    transport: Arc<SipUdpTransport>,
    transactions: Arc<TransactionManager>,
    state: Arc<Mutex<RegistrationState>>,
    current_cseq: Arc<Mutex<u32>>,
    call_id: String,
    timeout: Duration,
}

impl RegistrationManager {
    pub fn new(
        account: SipAccount,
        password: String,
        transport: Arc<SipUdpTransport>,
        transactions: Arc<TransactionManager>,
    ) -> Self {
        let call_id = format!("reg-{}@plasma-sip", uuid::Uuid::new_v4());
        Self {
            account,
            password: Zeroizing::new(password),
            transport,
            transactions,
            state: Arc::new(Mutex::new(RegistrationState::Unregistered)),
            current_cseq: Arc::new(Mutex::new(1)),
            call_id,
            timeout: Duration::from_secs(5),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn account(&self) -> &SipAccount {
        &self.account
    }

    pub async fn get_state(&self) -> RegistrationState {
        self.state.lock().await.clone()
    }

    /// Executes the SIP REGISTER sequence with challenge-response
    pub async fn register(&self, registrar_addr: SocketAddr) -> Result<(), String> {
        *self.state.lock().await = RegistrationState::Registering;
        info!(account_id = %self.account.id, "Initiating SIP registration");

        let cseq = {
            let mut c = self.current_cseq.lock().await;
            let val = *c;
            *c += 1;
            val
        };

        let branch1 = generate_branch();
        let from_tag = generate_tag();
        let local_ip = self.transport.local_ip_for(registrar_addr);
        let local_port = self.transport.local_addr().port();

        // 1. Send initial unauthenticated REGISTER
        let mut headers = HashMap::new();
        headers.insert(
            "Via".into(),
            format!(
                "SIP/2.0/UDP {}:{};branch={};rport",
                local_ip, local_port, branch1
            ),
        );
        headers.insert("Max-Forwards".into(), "70".into());
        headers.insert(
            "From".into(),
            format!("<{}>;tag={}", self.account.aor(), from_tag),
        );
        headers.insert("To".into(), format!("<{}>", self.account.aor()));
        headers.insert("Call-ID".into(), self.call_id.clone());
        headers.insert("CSeq".into(), format!("{} REGISTER", cseq));
        headers.insert(
            "Contact".into(),
            format!(
                "<sip:{}@{}:{}>;expires={}",
                self.account.username, local_ip, local_port, self.account.expires_secs
            ),
        );
        headers.insert("Expires".into(), self.account.expires_secs.to_string());
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());

        let req1 = SipRequest {
            method: SipMethod::Register,
            ruri: self.account.registrar_uri(),
            headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let key1 = TransactionKey {
            branch: branch1,
            method: "REGISTER".into(),
        };

        let mut rx1 = self.transactions.register_client_transaction(key1).await;
        let serialized1 = req1.serialize();

        // Await response with Timer E exponential backoff retransmission (RFC 3261 Section 17.1.2)
        let resp1 = match send_and_wait_with_retransmit(
            &self.transport,
            serialized1.as_bytes(),
            registrar_addr,
            &mut rx1,
            self.timeout,
            |_| {},
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                *self.state.lock().await = RegistrationState::Failed {
                    reason: e.clone(),
                    retry_in_secs: Some(30),
                };
                return Err(e);
            }
        };

        match resp1.status_code {
            200 => {
                info!("Registration succeeded immediately without challenge");
                let granted = extract_granted_expires(&resp1, self.account.expires_secs);
                *self.state.lock().await = RegistrationState::Registered {
                    expires_at_epoch_secs: current_epoch_secs() + granted,
                };
                return Ok(());
            }
            401 | 407 => {
                debug!(
                    "Received auth challenge {}: proceeding with Digest calculation",
                    resp1.status_code
                );
            }
            other => {
                let err_msg = format!(
                    "Registrar rejected with status {} {}",
                    other, resp1.reason_phrase
                );
                *self.state.lock().await = RegistrationState::Failed {
                    reason: err_msg.clone(),
                    retry_in_secs: Some(30),
                };
                return Err(err_msg);
            }
        }

        // 2. Compute digest response from challenge
        let challenge_header = resp1
            .headers
            .get("WWW-Authenticate")
            .or_else(|| resp1.headers.get("Proxy-Authenticate"))
            .ok_or_else(|| "Missing authenticate header in 401/407 challenge".to_string())?;

        let challenge = DigestChallenge::parse(challenge_header)
            .map_err(|e| format!("Failed to parse digest challenge: {:?}", e))?;

        let auth_val = DigestAuthorizer::build_authorization_header(
            &self.account.username,
            &self.password,
            &challenge,
            "REGISTER",
            &self.account.registrar_uri(),
            1,
        );

        // 3. Send authenticated REGISTER with incremented CSeq
        let cseq2 = {
            let mut c = self.current_cseq.lock().await;
            let val = *c;
            *c += 1;
            val
        };
        let branch2 = generate_branch();

        let mut headers2 = req1.headers.clone();
        headers2.insert(
            "Via".into(),
            format!(
                "SIP/2.0/UDP {}:{};branch={};rport",
                local_ip, local_port, branch2
            ),
        );
        headers2.insert("CSeq".into(), format!("{} REGISTER", cseq2));
        if resp1.status_code == 407 {
            headers2.insert("Proxy-Authorization".into(), auth_val);
        } else {
            headers2.insert("Authorization".into(), auth_val);
        }

        let req2 = SipRequest {
            method: SipMethod::Register,
            ruri: self.account.registrar_uri(),
            headers: headers2,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let key2 = TransactionKey {
            branch: branch2,
            method: "REGISTER".into(),
        };

        let mut rx2 = self.transactions.register_client_transaction(key2).await;
        let serialized2 = req2.serialize();

        let resp2 = match send_and_wait_with_retransmit(
            &self.transport,
            serialized2.as_bytes(),
            registrar_addr,
            &mut rx2,
            self.timeout,
            |_| {},
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                *self.state.lock().await = RegistrationState::Failed {
                    reason: e.clone(),
                    retry_in_secs: Some(60),
                };
                return Err(e);
            }
        };

        if resp2.status_code == 200 {
            info!("SIP Registration successfully completed with 200 OK");
            let granted = extract_granted_expires(&resp2, self.account.expires_secs);
            *self.state.lock().await = RegistrationState::Registered {
                expires_at_epoch_secs: current_epoch_secs() + granted,
            };
            Ok(())
        } else {
            let err_msg = format!(
                "Authenticated registration failed: {} {}",
                resp2.status_code, resp2.reason_phrase
            );
            *self.state.lock().await = RegistrationState::Failed {
                reason: err_msg.clone(),
                retry_in_secs: Some(60),
            };
            Err(err_msg)
        }
    }

    /// Executes the SIP unregistration sequence (Expires: 0)
    pub async fn unregister(&self, registrar_addr: SocketAddr) -> Result<(), String> {
        info!(account_id = %self.account.id, "Initiating SIP unregistration (Expires: 0)");

        let cseq = {
            let mut c = self.current_cseq.lock().await;
            let val = *c;
            *c += 1;
            val
        };

        let branch1 = generate_branch();
        let from_tag = generate_tag();
        let local_ip = self.transport.local_ip_for(registrar_addr);
        let local_port = self.transport.local_addr().port();

        let mut headers = HashMap::new();
        headers.insert(
            "Via".into(),
            format!(
                "SIP/2.0/UDP {}:{};branch={};rport",
                local_ip, local_port, branch1
            ),
        );
        headers.insert("Max-Forwards".into(), "70".into());
        headers.insert(
            "From".into(),
            format!("<{}>;tag={}", self.account.aor(), from_tag),
        );
        headers.insert("To".into(), format!("<{}>", self.account.aor()));
        headers.insert("Call-ID".into(), self.call_id.clone());
        headers.insert("CSeq".into(), format!("{} REGISTER", cseq));
        headers.insert(
            "Contact".into(),
            format!(
                "<sip:{}@{}:{}>;expires=0",
                self.account.username, local_ip, local_port
            ),
        );
        headers.insert("Expires".into(), "0".into());
        headers.insert("User-Agent".into(), "PlasmaSip/0.1".into());

        let req1 = SipRequest {
            method: SipMethod::Register,
            ruri: self.account.registrar_uri(),
            headers,
            raw_headers: Vec::new(),
            body: String::new(),
        };

        let key1 = TransactionKey {
            branch: branch1,
            method: "REGISTER".into(),
        };

        let mut rx1 = self.transactions.register_client_transaction(key1).await;
        let serialized1 = req1.serialize();

        let resp1 = send_and_wait_with_retransmit(
            &self.transport,
            serialized1.as_bytes(),
            registrar_addr,
            &mut rx1,
            self.timeout,
            |_| {},
        )
        .await
        .map_err(|e| format!("Network transport error: {}", e))?;

        if resp1.status_code == 200 {
            info!("SIP unregistration succeeded immediately with 200 OK");
            *self.state.lock().await = RegistrationState::Unregistered;
            return Ok(());
        }

        if resp1.status_code == 401 || resp1.status_code == 407 {
            let challenge_header = resp1
                .headers
                .get("WWW-Authenticate")
                .or_else(|| resp1.headers.get("Proxy-Authenticate"))
                .ok_or_else(|| {
                    "Missing authenticate header in unregistration challenge".to_string()
                })?;

            let challenge = DigestChallenge::parse(challenge_header)
                .map_err(|e| format!("Failed to parse digest challenge: {:?}", e))?;

            let auth_val = DigestAuthorizer::build_authorization_header(
                &self.account.username,
                &self.password,
                &challenge,
                "REGISTER",
                &self.account.registrar_uri(),
                1,
            );

            let cseq2 = {
                let mut c = self.current_cseq.lock().await;
                let val = *c;
                *c += 1;
                val
            };
            let branch2 = generate_branch();

            let mut headers2 = req1.headers.clone();
            headers2.insert(
                "Via".into(),
                format!(
                    "SIP/2.0/UDP {}:{};branch={};rport",
                    local_ip, local_port, branch2
                ),
            );
            headers2.insert("CSeq".into(), format!("{} REGISTER", cseq2));
            if resp1.status_code == 407 {
                headers2.insert("Proxy-Authorization".into(), auth_val);
            } else {
                headers2.insert("Authorization".into(), auth_val);
            }

            let req2 = SipRequest {
                method: SipMethod::Register,
                ruri: self.account.registrar_uri(),
                headers: headers2,
                raw_headers: Vec::new(),
                body: String::new(),
            };

            let key2 = TransactionKey {
                branch: branch2,
                method: "REGISTER".into(),
            };

            let mut rx2 = self.transactions.register_client_transaction(key2).await;
            let serialized2 = req2.serialize();

            let resp2 = send_and_wait_with_retransmit(
                &self.transport,
                serialized2.as_bytes(),
                registrar_addr,
                &mut rx2,
                self.timeout,
                |_| {},
            )
            .await
            .map_err(|e| format!("Network transport error: {}", e))?;

            if resp2.status_code == 200 {
                info!("Authenticated SIP unregistration succeeded with 200 OK");
                *self.state.lock().await = RegistrationState::Unregistered;
                return Ok(());
            }
        }

        *self.state.lock().await = RegistrationState::Unregistered;
        Ok(())
    }
}

/// Transmits a SIP UDP request with RFC 3261 Timer A / Timer E exponential backoff retransmission.
/// Ceases retransmissions upon receiving any 1xx provisional response, invoking `on_provisional`.
/// Returns the final (>= 200) response, or an error if timeout occurs.
pub async fn send_and_wait_with_retransmit<F>(
    transport: &SipUdpTransport,
    data: &[u8],
    target: SocketAddr,
    rx: &mut mpsc::Receiver<SipResponse>,
    timeout_dur: Duration,
    mut on_provisional: F,
) -> Result<SipResponse, String>
where
    F: FnMut(&SipResponse) + Send,
{
    let start = tokio::time::Instant::now();
    let mut attempt = 0;
    let mut retransmitting = true;

    // Send initial datagram
    transport
        .send_to(data, target)
        .await
        .map_err(|e| format!("Network transport error: {}", e))?;

    let mut next_retransmit = TransactionManager::calculate_retransmit_interval(attempt);

    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout_dur {
            return Err("Request timed out waiting for response".to_string());
        }
        let remaining = timeout_dur - elapsed;

        if retransmitting {
            let sleep_dur = next_retransmit.min(remaining);
            tokio::select! {
                recv_res = rx.recv() => {
                    match recv_res {
                        Some(resp) => {
                            if resp.status_code >= 200 {
                                return Ok(resp);
                            } else {
                                // Cease retransmission immediately on first 1xx provisional response (RFC 3261 §17.1.1.2)
                                retransmitting = false;
                                on_provisional(&resp);
                            }
                        }
                        None => return Err("Transaction channel closed".to_string()),
                    }
                }
                _ = tokio::time::sleep(sleep_dur) => {
                    attempt += 1;
                    next_retransmit = TransactionManager::calculate_retransmit_interval(attempt);
                    trace!(attempt = attempt, "Retransmitting SIP UDP datagram");
                    let _ = transport.send_to(data, target).await;
                }
            }
        } else {
            // Already received 1xx, wait for final response without retransmission
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(resp)) => {
                    if resp.status_code >= 200 {
                        return Ok(resp);
                    }
                    on_provisional(&resp);
                }
                Ok(None) => return Err("Transaction channel closed".to_string()),
                Err(_) => return Err("Request timed out waiting for final response".to_string()),
            }
        }
    }
}

fn extract_granted_expires(resp: &SipResponse, configured_expires: u32) -> u64 {
    // 1. Check Contact headers for ;expires= parameter (RFC 3261 Section 10.2.4: Contact expires takes precedence over Expires header)
    for contact in resp.get_headers("Contact") {
        for param in contact.split(';') {
            let trimmed = param.trim();
            if let Some(rest) = trimmed.strip_prefix("expires=") {
                let clean = rest.trim_matches('"');
                if let Ok(secs) = clean.parse::<u64>() {
                    return secs;
                }
            }
        }
    }

    // 2. Check top-level Expires header
    if let Some(val) = resp.headers.get("Expires") {
        if let Ok(secs) = val.trim().parse::<u64>() {
            return secs;
        }
    }

    configured_expires as u64
}

fn current_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
