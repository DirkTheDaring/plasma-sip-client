//! RFC 3261 Section 17 Transaction Management.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};

use crate::message::{SipRequest, SipResponse};

pub const DEFAULT_T1: Duration = Duration::from_millis(500);
pub const DEFAULT_T2: Duration = Duration::from_millis(4000);
pub const DEFAULT_T4: Duration = Duration::from_millis(5000);

/// Timer B and Timer F: Transaction timeout (64 * T1 = 32 seconds)
pub const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TransactionKey {
    pub branch: String,
    pub method: String,
}

impl TransactionKey {
    pub fn from_request(req: &SipRequest) -> Option<Self> {
        let via = req.headers.get("Via")?;
        let branch = extract_branch(via)?;
        Some(Self {
            branch,
            method: req.method.to_string(),
        })
    }

    pub fn from_response(res: &SipResponse) -> Option<Self> {
        let via = res.headers.get("Via")?;
        let branch = extract_branch(via)?;
        let cseq = res.headers.get("CSeq")?;
        let method = cseq.split_whitespace().nth(1)?;
        Some(Self {
            branch,
            method: method.to_string(),
        })
    }
}

#[derive(Debug)]
struct TransactionEntry {
    sender: mpsc::Sender<SipResponse>,
    created_at: Instant,
}

#[derive(Clone)]
pub struct TransactionManager {
    pending: Arc<Mutex<HashMap<TransactionKey, TransactionEntry>>>,
}

impl TransactionManager {
    pub fn new() -> Self {
        let pending = Arc::new(Mutex::new(
            HashMap::<TransactionKey, TransactionEntry>::new(),
        ));
        let pending_clone = Arc::clone(&pending);

        // Spawn background transaction reaper to prevent memory leaks from unresponded transactions (RFC 3261 Timer B/F)
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                let mut map = pending_clone.lock().await;
                map.retain(|_, entry| entry.created_at.elapsed() < TRANSACTION_TIMEOUT);
            }
        });

        Self { pending }
    }

    /// Calculates RFC 3261 Timer A / Timer E exponential backoff interval for UDP retransmissions.
    ///
    /// Initially T1 (500ms), doubles on each subsequent retransmission up to T2 (4000ms):
    /// Attempt 0: 500ms
    /// Attempt 1: 1000ms
    /// Attempt 2: 2000ms
    /// Attempt >= 3: 4000ms
    pub fn calculate_retransmit_interval(attempt: u32) -> Duration {
        let multiplier = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
        let interval_ms = (DEFAULT_T1.as_millis() as u64).saturating_mul(multiplier);
        Duration::from_millis(interval_ms.min(DEFAULT_T2.as_millis() as u64))
    }

    /// Registers a pending client transaction and returns an MPSC receiver for provisional (1xx) and final (>=2xx) responses.
    pub async fn register_client_transaction(
        &self,
        key: TransactionKey,
    ) -> mpsc::Receiver<SipResponse> {
        let (tx, rx) = mpsc::channel(16);
        let mut map = self.pending.lock().await;
        map.insert(
            key,
            TransactionEntry {
                sender: tx,
                created_at: Instant::now(),
            },
        );
        rx
    }

    /// Delivers an incoming response to the matching transaction.
    /// Provisional responses (1xx) keep the transaction active.
    /// Final responses (>=200) terminate the transaction and remove it from the map.
    pub async fn handle_response(&self, res: SipResponse) -> bool {
        if let Some(key) = TransactionKey::from_response(&res) {
            let mut map = self.pending.lock().await;
            if res.status_code < 200 {
                if let Some(entry) = map.get(&key) {
                    let _ = entry.sender.send(res).await;
                    return true;
                }
            } else if let Some(entry) = map.remove(&key) {
                let _ = entry.sender.send(res).await;
                return true;
            }
        }
        false
    }

    /// Explicitly prunes transactions older than `max_age` and returns the count of evicted transactions.
    pub async fn prune_stale(&self, max_age: Duration) -> usize {
        let mut map = self.pending.lock().await;
        let initial_len = map.len();
        map.retain(|_, entry| entry.created_at.elapsed() < max_age);
        initial_len - map.len()
    }

    /// Returns the number of currently active pending transactions.
    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn extract_branch(via: &str) -> Option<String> {
    for part in via.split(';') {
        let trimmed = part.trim();
        if let Some(branch) = trimmed.strip_prefix("branch=") {
            return Some(branch.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_branch() {
        let via = "SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bKabcdef12345;rport";
        assert_eq!(extract_branch(via), Some("z9hG4bKabcdef12345".to_string()));
    }

    #[test]
    fn test_calculate_retransmit_interval() {
        assert_eq!(
            TransactionManager::calculate_retransmit_interval(0),
            Duration::from_millis(500)
        );
        assert_eq!(
            TransactionManager::calculate_retransmit_interval(1),
            Duration::from_millis(1000)
        );
        assert_eq!(
            TransactionManager::calculate_retransmit_interval(2),
            Duration::from_millis(2000)
        );
        assert_eq!(
            TransactionManager::calculate_retransmit_interval(3),
            Duration::from_millis(4000)
        );
        assert_eq!(
            TransactionManager::calculate_retransmit_interval(4),
            Duration::from_millis(4000)
        );
    }

    #[tokio::test]
    async fn test_transaction_lifecycle_and_prune() {
        let tm = TransactionManager::new();
        let key = TransactionKey {
            branch: "z9hG4bK-test1".into(),
            method: "INVITE".into(),
        };

        let mut rx = tm.register_client_transaction(key.clone()).await;
        assert_eq!(tm.pending_count().await, 1);

        // Provisional 180 Ringing does not terminate transaction
        let prov_resp = SipResponse {
            status_code: 180,
            reason_phrase: "Ringing".into(),
            headers: [
                (
                    "Via".into(),
                    "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-test1".into(),
                ),
                ("CSeq".into(), "1 INVITE".into()),
            ]
            .into_iter()
            .collect(),
            raw_headers: Vec::new(),
            body: String::new(),
        };
        assert!(tm.handle_response(prov_resp).await);
        assert_eq!(tm.pending_count().await, 1);
        assert!(rx.recv().await.is_some());

        // Final 200 OK terminates transaction
        let final_resp = SipResponse {
            status_code: 200,
            reason_phrase: "OK".into(),
            headers: [
                (
                    "Via".into(),
                    "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-test1".into(),
                ),
                ("CSeq".into(), "1 INVITE".into()),
            ]
            .into_iter()
            .collect(),
            raw_headers: Vec::new(),
            body: String::new(),
        };
        assert!(tm.handle_response(final_resp).await);
        assert_eq!(tm.pending_count().await, 0);
        assert!(rx.recv().await.is_some());

        // Register another and verify prune_stale
        let key2 = TransactionKey {
            branch: "z9hG4bK-test2".into(),
            method: "REGISTER".into(),
        };
        let _rx2 = tm.register_client_transaction(key2).await;
        assert_eq!(tm.pending_count().await, 1);

        // Max age of 0 removes immediately
        let pruned = tm.prune_stale(Duration::from_millis(0)).await;
        assert_eq!(pruned, 1);
        assert_eq!(tm.pending_count().await, 0);
    }
}
