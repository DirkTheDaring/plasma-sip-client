//! Account manager coordinating stored accounts, network SIP registration, and renewals.

use sip_domain::{AccountId, RegistrationState, SipAccount};
use sip_engine::RegistrationManager;
use sip_protocol::TransactionManager;
use sip_storage::StorageManager;
use sip_transport::SipUdpTransport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

pub struct AccountManager {
    storage: Arc<StorageManager>,
    transport: Arc<SipUdpTransport>,
    transactions: Arc<TransactionManager>,
    registration_states: Arc<Mutex<HashMap<AccountId, RegistrationState>>>,
    registration_managers: Arc<Mutex<HashMap<AccountId, Arc<RegistrationManager>>>>,
    renewal_tasks: Arc<Mutex<HashMap<AccountId, tokio::task::JoinHandle<()>>>>,
    state_tx: tokio::sync::broadcast::Sender<(AccountId, RegistrationState)>,
}

impl AccountManager {
    pub fn new(
        storage: Arc<StorageManager>,
        transport: Arc<SipUdpTransport>,
        transactions: Arc<TransactionManager>,
    ) -> Self {
        let (state_tx, _) = tokio::sync::broadcast::channel(128);
        Self {
            storage,
            transport,
            transactions,
            registration_states: Arc::new(Mutex::new(HashMap::new())),
            registration_managers: Arc::new(Mutex::new(HashMap::new())),
            renewal_tasks: Arc::new(Mutex::new(HashMap::new())),
            state_tx,
        }
    }

    /// Subscribes to registration state transition events across all accounts.
    pub fn subscribe_registration_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<(AccountId, RegistrationState)> {
        self.state_tx.subscribe()
    }

    pub fn list_accounts(&self) -> Result<Vec<SipAccount>, String> {
        self.storage
            .list_accounts()
            .map_err(|e| format!("Database query error: {}", e))
    }

    pub fn save_account(&self, account: &SipAccount, password: &str) -> Result<(), String> {
        account.validate().map_err(|e| e.to_string())?;
        self.storage
            .save_account(account)
            .map_err(|e| format!("Failed to save account: {}", e))?;
        self.storage
            .save_account_secret(account.id, password)
            .map_err(|e| format!("Failed to store secret: {}", e))?;
        Ok(())
    }

    pub async fn delete_account(&self, id: AccountId) -> Result<(), String> {
        // First unregister cleanly over SIP network if registered
        let _ = self.unregister_account(id).await;

        self.storage
            .delete_account(id)
            .map_err(|e| format!("Failed to delete account: {}", e))?;
        let _ = self.storage.delete_account_secret(id);
        self.registration_states.lock().await.remove(&id);
        self.registration_managers.lock().await.remove(&id);
        let _ = self.state_tx.send((id, RegistrationState::Unregistered));
        Ok(())
    }

    pub async fn get_registration_state(&self, id: AccountId) -> RegistrationState {
        self.registration_states
            .lock()
            .await
            .get(&id)
            .cloned()
            .unwrap_or(RegistrationState::Unregistered)
    }

    pub async fn set_registration_state(&self, id: AccountId, state: RegistrationState) {
        self.registration_states
            .lock()
            .await
            .insert(id, state.clone());
        let _ = self.state_tx.send((id, state));
    }

    /// Spawns a background retry task that attempts registration with backoff until successful.
    async fn spawn_retry_task(&self, id: AccountId, initial_delay_secs: u64) {
        let mut tasks = self.renewal_tasks.lock().await;
        if tasks.contains_key(&id) {
            return;
        }

        let storage = self.storage.clone();
        let transport = self.transport.clone();
        let transactions = self.transactions.clone();
        let registration_managers = self.registration_managers.clone();
        let registration_states = self.registration_states.clone();
        let state_tx = self.state_tx.clone();

        let retry_handle = tokio::spawn(async move {
            let mut delay = initial_delay_secs;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;

                // Check if account still exists and is enabled
                let account = match storage.get_account(id) {
                    Ok(Some(acc)) if acc.enabled => acc,
                    _ => {
                        info!(account_id = %id, "Account disabled or removed; stopping retry task");
                        break;
                    }
                };

                // Check credentials
                let password = match storage.get_account_secret(id) {
                    Ok(Some(pw)) => pw,
                    _ => {
                        warn!(account_id = %id, "Retrying registration: credentials not yet available, will retry");
                        delay = (delay * 2).min(30);
                        continue;
                    }
                };

                // Resolve DNS
                let registrar_addr = match sip_transport::resolve_sip_target(&account.registrar, account.registrar_port).await {
                    Ok(addrs) => match transport.select_compatible_addr(addrs) {
                        Some(addr) => addr,
                        None => {
                            warn!(account_id = %id, "Retrying registration: no compatible DNS addresses, will retry");
                            delay = (delay * 2).min(30);
                            continue;
                        }
                    },
                    Err(_) => {
                        warn!(account_id = %id, "Retrying registration: DNS resolution failed, will retry");
                        delay = (delay * 2).min(30);
                        continue;
                    }
                };

                let reg_mgr = {
                    let mut managers = registration_managers.lock().await;
                    if let Some(existing) = managers.get(&id) {
                        if existing.account().id == account.id
                            && existing.account().username == account.username
                            && existing.account().registrar == account.registrar
                        {
                            existing.clone()
                        } else {
                            let m = Arc::new(RegistrationManager::new(
                                account.clone(),
                                password,
                                transport.clone(),
                                transactions.clone(),
                            ));
                            managers.insert(id, m.clone());
                            m
                        }
                    } else {
                        let m = Arc::new(RegistrationManager::new(
                            account.clone(),
                            password,
                            transport.clone(),
                            transactions.clone(),
                        ));
                        managers.insert(id, m.clone());
                        m
                    }
                };

                info!(account_id = %id, "Attempting background retry registration");
                match reg_mgr.register(registrar_addr).await {
                    Ok(()) => {
                        info!(account_id = %id, "Background retry registration succeeded with 200 OK!");
                        let st = reg_mgr.get_state().await;
                        registration_states.lock().await.insert(id, st.clone());
                        let _ = state_tx.send((id, st));

                        // Seamlessly transition into the renewal loop
                        loop {
                            let current_st = reg_mgr.get_state().await;
                            let sleep_secs = match current_st {
                                RegistrationState::Registered {
                                    expires_at_epoch_secs,
                                } => {
                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0);
                                    let remaining = expires_at_epoch_secs.saturating_sub(now);
                                    remaining.saturating_sub(30).max(10)
                                }
                                RegistrationState::Failed { retry_in_secs, .. } => {
                                    retry_in_secs.unwrap_or(30) as u64
                                }
                                _ => (account.expires_secs as u64).saturating_sub(30).max(10),
                            };
                            tokio::time::sleep(tokio::time::Duration::from_secs(sleep_secs)).await;

                            if let Ok(Some(acc)) = storage.get_account(id) {
                                if !acc.enabled {
                                    break;
                                }
                            } else {
                                break;
                            }

                            info!(account_id = %id, "Triggering automatic SIP registration renewal");
                            match reg_mgr.register(registrar_addr).await {
                                Ok(()) => {
                                    let st = reg_mgr.get_state().await;
                                    registration_states.lock().await.insert(id, st.clone());
                                    let _ = state_tx.send((id, st));
                                }
                                Err(_) => {
                                    warn!(account_id = %id, "Registration renewal failed");
                                    let st = reg_mgr.get_state().await;
                                    registration_states.lock().await.insert(id, st.clone());
                                    let _ = state_tx.send((id, st));
                                }
                            }
                        }
                        break;
                    }
                    Err(e) => {
                        warn!(account_id = %id, "Background retry registration failed; will retry again");
                        let mut st = reg_mgr.get_state().await;
                        if st == RegistrationState::Unregistered || st == RegistrationState::Registering {
                            st = RegistrationState::Failed {
                                reason: e.clone(),
                                retry_in_secs: Some(delay as u32),
                            };
                        }
                        registration_states.lock().await.insert(id, st.clone());
                        let _ = state_tx.send((id, st));
                        delay = (delay * 2).min(30);
                    }
                }
            }
        });

        tasks.insert(id, retry_handle);
    }

    /// Explicitly triggers real RFC 3261 SIP registration over the network for an account.
    pub async fn register_account(&self, id: AccountId) -> Result<(), String> {
        let account = self
            .storage
            .get_account(id)
            .map_err(|e| format!("Database error: {}", e))?
            .ok_or_else(|| format!("Account {} not found", id))?;

        if !account.enabled {
            return Err(format!("Account {} is disabled", id));
        }

        // Abort previous renewal or retry task if running
        {
            let mut tasks = self.renewal_tasks.lock().await;
            if let Some(task) = tasks.remove(&id) {
                task.abort();
            }
        }

        let password = match self.storage.get_account_secret(id) {
            Ok(Some(pw)) => pw,
            Ok(None) => {
                let err = format!("No credentials found for account {}", id);
                self.set_registration_state(
                    id,
                    RegistrationState::Failed {
                        reason: err.clone(),
                        retry_in_secs: Some(15),
                    },
                )
                .await;
                self.spawn_retry_task(id, 5).await;
                return Err(err);
            }
            Err(e) => {
                let err = format!("Failed to read credentials for account {}: {}", id, e);
                self.set_registration_state(
                    id,
                    RegistrationState::Failed {
                        reason: err.clone(),
                        retry_in_secs: Some(15),
                    },
                )
                .await;
                self.spawn_retry_task(id, 5).await;
                return Err(err);
            }
        };

        // RFC 3263 DNS resolution of registrar hostname and port
        let registrar_addr =
            match sip_transport::resolve_sip_target(&account.registrar, account.registrar_port)
                .await
            {
                Ok(addrs) => match self.transport.select_compatible_addr(addrs) {
                    Some(addr) => addr,
                    None => {
                        let err = "DNS lookup for registrar returned no compatible addresses".to_string();
                        self.set_registration_state(
                            id,
                            RegistrationState::Failed {
                                reason: err.clone(),
                                retry_in_secs: Some(15),
                            },
                        )
                        .await;
                        self.spawn_retry_task(id, 5).await;
                        return Err(err);
                    }
                },
                Err(e) => {
                    let err = format!("DNS resolution error for registrar: {}", e);
                    self.set_registration_state(
                        id,
                        RegistrationState::Failed {
                            reason: err.clone(),
                            retry_in_secs: Some(15),
                        },
                    )
                    .await;
                    self.spawn_retry_task(id, 5).await;
                    return Err(err);
                }
            };

        self.set_registration_state(id, RegistrationState::Registering)
            .await;

        let reg_mgr = {
            let mut managers = self.registration_managers.lock().await;
            if let Some(existing) = managers.get(&id) {
                if existing.account().id == account.id
                    && existing.account().username == account.username
                    && existing.account().registrar == account.registrar
                {
                    existing.clone()
                } else {
                    let m = Arc::new(RegistrationManager::new(
                        account.clone(),
                        password,
                        self.transport.clone(),
                        self.transactions.clone(),
                    ));
                    managers.insert(id, m.clone());
                    m
                }
            } else {
                let m = Arc::new(RegistrationManager::new(
                    account.clone(),
                    password,
                    self.transport.clone(),
                    self.transactions.clone(),
                ));
                managers.insert(id, m.clone());
                m
            }
        };

        match reg_mgr.register(registrar_addr).await {
            Ok(()) => {
                let current_state = reg_mgr.get_state().await;
                self.set_registration_state(id, current_state.clone()).await;

                // Spawn background auto-renewal task before expires_secs
                let states_clone = self.registration_states.clone();
                let state_tx_clone = self.state_tx.clone();
                let reg_mgr_clone = reg_mgr.clone();
                let expires_secs = account.expires_secs;

                let renewal_handle = tokio::spawn(async move {
                    loop {
                        let current_st = reg_mgr_clone.get_state().await;
                        let sleep_secs = match current_st {
                            RegistrationState::Registered {
                                expires_at_epoch_secs,
                            } => {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                let remaining = expires_at_epoch_secs.saturating_sub(now);
                                remaining.saturating_sub(30).max(10)
                            }
                            RegistrationState::Failed { retry_in_secs, .. } => {
                                retry_in_secs.unwrap_or(30) as u64
                            }
                            _ => (expires_secs as u64).saturating_sub(30).max(10),
                        };
                        tokio::time::sleep(tokio::time::Duration::from_secs(sleep_secs)).await;

                        info!(account_id = %id, "Triggering automatic SIP registration renewal");
                        match reg_mgr_clone.register(registrar_addr).await {
                            Ok(()) => {
                                let st = reg_mgr_clone.get_state().await;
                                states_clone.lock().await.insert(id, st.clone());
                                let _ = state_tx_clone.send((id, st));
                            }
                            Err(_) => {
                                warn!(account_id = %id, "Registration renewal failed");
                                let st = reg_mgr_clone.get_state().await;
                                states_clone.lock().await.insert(id, st.clone());
                                let _ = state_tx_clone.send((id, st));
                            }
                        }
                    }
                });

                self.renewal_tasks.lock().await.insert(id, renewal_handle);
                Ok(())
            }
            Err(e) => {
                let mut st = reg_mgr.get_state().await;
                if st == RegistrationState::Unregistered || st == RegistrationState::Registering {
                    st = RegistrationState::Failed {
                        reason: e.clone(),
                        retry_in_secs: Some(15),
                    };
                }
                self.set_registration_state(id, st).await;
                self.spawn_retry_task(id, 5).await;
                Err(e)
            }
        }
    }

    /// Explicitly unregisters an account (Expires: 0) over the network.
    pub async fn unregister_account(&self, id: AccountId) -> Result<(), String> {
        // Abort background renewal task
        {
            let mut tasks = self.renewal_tasks.lock().await;
            if let Some(task) = tasks.remove(&id) {
                task.abort();
            }
        }

        let reg_mgr_opt = self.registration_managers.lock().await.remove(&id);

        let account = match self.storage.get_account(id) {
            Ok(Some(acc)) => acc,
            Ok(None) => {
                self.set_registration_state(id, RegistrationState::Unregistered)
                    .await;
                return Ok(());
            }
            Err(e) => return Err(format!("Database error: {}", e)),
        };

        let password = self
            .storage
            .get_account_secret(id)
            .unwrap_or_default()
            .unwrap_or_default();

        if let Ok(addrs) =
            sip_transport::resolve_sip_target(&account.registrar, account.registrar_port).await
        {
            if let Some(registrar_addr) = self.transport.select_compatible_addr(addrs) {
                let reg_mgr = reg_mgr_opt.unwrap_or_else(|| {
                    Arc::new(RegistrationManager::new(
                        account,
                        password,
                        self.transport.clone(),
                        self.transactions.clone(),
                    ))
                });
                let _ = reg_mgr.unregister(registrar_addr).await;
            }
        }

        self.set_registration_state(id, RegistrationState::Unregistered)
            .await;
        Ok(())
    }

    /// Recovers all enabled accounts from database and triggers real network registration.
    pub async fn recover_and_register_all(&self) -> Result<usize, String> {
        let accounts = self.list_accounts()?;
        let mut recovered = 0;
        for acc in accounts {
            if acc.enabled {
                info!(account_id = %acc.id, "Recovering and registering enabled SIP account");
                match self.register_account(acc.id).await {
                    Ok(()) => {
                        recovered += 1;
                    }
                    Err(_) => {
                        warn!(
                            account_id = %acc.id,
                            "Failed to register account during startup recovery (background retry scheduled)"
                        );
                    }
                }
            }
        }
        Ok(recovered)
    }

    /// Reconnects all enabled accounts immediately.
    pub async fn reconnect_all(&self) -> Result<usize, String> {
        self.recover_and_register_all().await
    }
}
