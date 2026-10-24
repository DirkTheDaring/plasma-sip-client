//! SQLite and Secret Service storage for Plasma SIP.

use chrono::{DateTime, Utc};
use keyring::Entry;
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use uuid::Uuid;

use sip_domain::{
    AccountId, AudioConfig, CallDirection, CallId, CallRejectMode, Contact, ContactId,
    ContactNumber, ContactSource, NumberType, SipAccount, TransportProtocol,
    normalize_number_digits, text_to_t9,
};

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Keyring secret service error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("Account not found: {0}")]
    AccountNotFound(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

#[derive(Debug, Clone)]
pub struct CallLogEntry {
    pub id: CallId,
    pub account_id: AccountId,
    pub direction: CallDirection,
    pub remote_party: String,
    pub started_at: DateTime<Utc>,
    pub duration_seconds: u32,
    pub disconnect_reason: String,
}

pub trait CredentialBackend: Send + Sync {
    fn save_secret(&self, id: AccountId, password: &str) -> Result<(), StorageError>;
    fn get_secret(&self, id: AccountId) -> Result<Option<String>, StorageError>;
    fn delete_secret(&self, id: AccountId) -> Result<(), StorageError>;
    fn is_isolated(&self) -> bool;
}

fn is_running_under_test() -> bool {
    #[cfg(test)]
    {
        true
    }
    #[cfg(not(test))]
    {
        if std::env::var_os("RUST_TEST_THREADS").is_some()
            || std::env::var_os("PLASMA_SIP_TEST_MODE").is_some()
        {
            return true;
        }
        if let Ok(exe) = std::env::current_exe() {
            let s = exe.to_string_lossy();
            if s.contains("/deps/") || s.contains("test") {
                return true;
            }
        }
        false
    }
}

#[derive(Default, Debug)]
pub struct KeyringCredentialBackend;

impl CredentialBackend for KeyringCredentialBackend {
    fn save_secret(&self, id: AccountId, password: &str) -> Result<(), StorageError> {
        if is_running_under_test() {
            return Err(StorageError::Keyring(keyring::Error::PlatformFailure(
                "Safety guard: Host keyring access is strictly prohibited during test execution."
                    .into(),
            )));
        }
        let service = "org.kde.PlasmaSip";
        let user = id.to_string();
        let entry = Entry::new(service, &user)?;
        entry.set_password(password)?;
        Ok(())
    }

    fn get_secret(&self, id: AccountId) -> Result<Option<String>, StorageError> {
        if is_running_under_test() {
            return Err(StorageError::Keyring(keyring::Error::PlatformFailure(
                "Safety guard: Host keyring access is strictly prohibited during test execution."
                    .into(),
            )));
        }
        let service = "org.kde.PlasmaSip";
        let user = id.to_string();
        let entry = Entry::new(service, &user)?;
        match entry.get_password() {
            Ok(pw) => Ok(Some(pw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(StorageError::Keyring(e)),
        }
    }

    fn delete_secret(&self, id: AccountId) -> Result<(), StorageError> {
        if is_running_under_test() {
            return Err(StorageError::Keyring(keyring::Error::PlatformFailure(
                "Safety guard: Host keyring access is strictly prohibited during test execution."
                    .into(),
            )));
        }
        let service = "org.kde.PlasmaSip";
        let user = id.to_string();
        let entry = Entry::new(service, &user)?;
        match entry.delete_credential() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StorageError::Keyring(e)),
        }
    }

    fn is_isolated(&self) -> bool {
        false
    }
}

pub struct InMemoryCredentialBackend {
    store: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
}

impl Default for InMemoryCredentialBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryCredentialBackend {
    pub fn new() -> Self {
        Self {
            store: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }
}

impl CredentialBackend for InMemoryCredentialBackend {
    fn save_secret(&self, id: AccountId, password: &str) -> Result<(), StorageError> {
        let mut store = self.store.lock().unwrap();
        store.insert(id.to_string(), password.to_string());
        Ok(())
    }

    fn get_secret(&self, id: AccountId) -> Result<Option<String>, StorageError> {
        let store = self.store.lock().unwrap();
        Ok(store.get(&id.to_string()).cloned())
    }

    fn delete_secret(&self, id: AccountId) -> Result<(), StorageError> {
        let mut store = self.store.lock().unwrap();
        store.remove(&id.to_string());
        Ok(())
    }

    fn is_isolated(&self) -> bool {
        true
    }
}

pub struct StorageManager {
    conn: Arc<Mutex<Connection>>,
    backend: Arc<dyn CredentialBackend>,
}

impl StorageManager {
    pub fn new_in_memory() -> Result<Self, StorageError> {
        let backend: Arc<dyn CredentialBackend> = Arc::new(InMemoryCredentialBackend::new());
        let conn = Connection::open_in_memory()?;
        let manager = Self {
            conn: Arc::new(Mutex::new(conn)),
            backend,
        };
        manager.migrate()?;
        Ok(manager)
    }

    pub fn new_with_backend<P: AsRef<Path>>(
        path: P,
        backend: Arc<dyn CredentialBackend>,
    ) -> Result<Self, StorageError> {
        let conn = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.is_file() {
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
        let manager = Self {
            conn: Arc::new(Mutex::new(conn)),
            backend,
        };
        manager.migrate()?;
        Ok(manager)
    }

    pub fn new_with_path<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        Self::new_with_backend(path, Arc::new(KeyringCredentialBackend))
    }

    pub fn is_isolated_backend(&self) -> bool {
        self.backend.is_isolated()
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        let user_version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if user_version < 1 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS accounts (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    username TEXT NOT NULL,
                    display_name TEXT,
                    domain TEXT NOT NULL,
                    registrar TEXT NOT NULL,
                    registrar_port INTEGER NOT NULL,
                    transport TEXT NOT NULL,
                    expires_secs INTEGER NOT NULL,
                    enabled INTEGER NOT NULL,
                    is_default INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS call_history (
                    id TEXT PRIMARY KEY,
                    account_id TEXT NOT NULL,
                    direction TEXT NOT NULL,
                    remote_party TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    duration_seconds INTEGER NOT NULL,
                    disconnect_reason TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                PRAGMA user_version = 1;
                "#,
            )?;
        }

        if user_version < 2 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS account_credentials (
                    account_id TEXT PRIMARY KEY,
                    password TEXT NOT NULL
                );
                PRAGMA user_version = 2;
                "#,
            )?;
        }

        if user_version < 3 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS contacts (
                    id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    source TEXT NOT NULL,
                    fritzbox_unique_id TEXT,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS contact_numbers (
                    id TEXT PRIMARY KEY,
                    contact_id TEXT NOT NULL,
                    raw_number TEXT NOT NULL,
                    normalized_digits TEXT NOT NULL,
                    number_type TEXT NOT NULL,
                    quickdial TEXT,
                    vanity TEXT,
                    FOREIGN KEY(contact_id) REFERENCES contacts(id) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_contact_numbers_norm ON contact_numbers(normalized_digits);
                CREATE INDEX IF NOT EXISTS idx_contact_numbers_cid ON contact_numbers(contact_id);
                CREATE INDEX IF NOT EXISTS idx_contacts_name ON contacts(display_name);
                PRAGMA user_version = 3;
                "#,
            )?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, StorageError> {
        let conn = self.conn.lock().unwrap();
        let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(version)
    }

    // Account persistence
    pub fn save_account(&self, account: &SipAccount) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        let transport_str = match account.transport {
            TransportProtocol::Udp => "udp",
            TransportProtocol::Tcp => "tcp",
            TransportProtocol::Tls => "tls",
        };

        conn.execute(
            r#"
            INSERT OR REPLACE INTO accounts (
                id, name, username, display_name, domain, registrar, registrar_port,
                transport, expires_secs, enabled, is_default
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11);
            "#,
            params![
                account.id.to_string(),
                account.name,
                account.username,
                account.display_name,
                account.domain,
                account.registrar,
                account.registrar_port,
                transport_str,
                account.expires_secs,
                if account.enabled { 1 } else { 0 },
                if account.is_default { 1 } else { 0 },
            ],
        )?;
        Ok(())
    }

    pub fn list_accounts(&self) -> Result<Vec<SipAccount>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, username, display_name, domain, registrar, registrar_port, transport, expires_secs, enabled, is_default FROM accounts",
        )?;

        let rows = stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let id = AccountId(Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()));
            let transport_str: String = row.get(7)?;
            let transport = match transport_str.as_str() {
                "tcp" => TransportProtocol::Tcp,
                "tls" => TransportProtocol::Tls,
                _ => TransportProtocol::Udp,
            };
            let enabled_int: i32 = row.get(9)?;
            let default_int: i32 = row.get(10)?;

            Ok(SipAccount {
                id,
                name: row.get(1)?,
                username: row.get(2)?,
                display_name: row.get(3)?,
                domain: row.get(4)?,
                registrar: row.get(5)?,
                registrar_port: row.get(6)?,
                transport,
                expires_secs: row.get(8)?,
                enabled: enabled_int != 0,
                is_default: default_int != 0,
            })
        })?;

        let mut accounts = Vec::new();
        for r in rows {
            accounts.push(r?);
        }
        Ok(accounts)
    }

    pub fn get_account(&self, id: AccountId) -> Result<Option<SipAccount>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, username, display_name, domain, registrar, registrar_port, transport, expires_secs, enabled, is_default FROM accounts WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![id.to_string()], |row| {
            let id_str: String = row.get(0)?;
            let id = AccountId(Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()));
            let transport_str: String = row.get(7)?;
            let transport = match transport_str.as_str() {
                "tcp" => TransportProtocol::Tcp,
                "tls" => TransportProtocol::Tls,
                _ => TransportProtocol::Udp,
            };
            let enabled_int: i32 = row.get(9)?;
            let default_int: i32 = row.get(10)?;

            Ok(SipAccount {
                id,
                name: row.get(1)?,
                username: row.get(2)?,
                display_name: row.get(3)?,
                domain: row.get(4)?,
                registrar: row.get(5)?,
                registrar_port: row.get(6)?,
                transport,
                expires_secs: row.get(8)?,
                enabled: enabled_int != 0,
                is_default: default_int != 0,
            })
        })?;

        if let Some(row) = rows.next() {
            Ok(Some(row?))
        } else {
            Ok(None)
        }
    }

    pub fn delete_account(&self, id: AccountId) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM accounts WHERE id = ?1",
            params![id.to_string()],
        )?;
        drop(conn);
        let _ = self.delete_account_secret(id);
        Ok(())
    }

    // Call History
    pub fn log_call(&self, entry: &CallLogEntry) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        let dir_str = match entry.direction {
            CallDirection::Inbound => "inbound",
            CallDirection::Outbound => "outbound",
        };

        conn.execute(
            r#"
            INSERT INTO call_history (id, account_id, direction, remote_party, started_at, duration_seconds, disconnect_reason)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);
            "#,
            params![
                entry.id.to_string(),
                entry.account_id.to_string(),
                dir_str,
                entry.remote_party,
                entry.started_at.to_rfc3339(),
                entry.duration_seconds,
                entry.disconnect_reason,
            ],
        )?;
        Ok(())
    }

    pub fn list_call_history(&self, limit: usize) -> Result<Vec<CallLogEntry>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, direction, remote_party, started_at, duration_seconds, disconnect_reason FROM call_history ORDER BY started_at DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let id_str: String = row.get(0)?;
            let id = CallId(Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()));
            let acc_str: String = row.get(1)?;
            let account_id =
                AccountId(Uuid::parse_str(&acc_str).unwrap_or_else(|_| Uuid::new_v4()));
            let dir_str: String = row.get(2)?;
            let direction = if dir_str == "inbound" {
                CallDirection::Inbound
            } else {
                CallDirection::Outbound
            };
            let time_str: String = row.get(4)?;
            let started_at = DateTime::parse_from_rfc3339(&time_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(CallLogEntry {
                id,
                account_id,
                direction,
                remote_party: row.get(3)?,
                started_at,
                duration_seconds: row.get(5)?,
                disconnect_reason: row.get(6)?,
            })
        })?;

        let mut history = Vec::new();
        for r in rows {
            history.push(r?);
        }
        Ok(history)
    }

    pub fn clear_call_history(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM call_history", [])?;
        Ok(())
    }

    pub fn delete_call_log(&self, id: &CallId) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM call_history WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Stores account credentials persistently in SQLite and synchronizes to Keyring (best-effort).
    pub fn save_account_secret(&self, id: AccountId, password: &str) -> Result<(), StorageError> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO account_credentials (account_id, password) VALUES (?1, ?2);",
                params![id.to_string(), password],
            )?;
        }
        let _ = self.backend.save_secret(id, password);
        Ok(())
    }

    /// Retrieves account credentials from Keyring, or falls back to SQLite persistent storage.
    pub fn get_account_secret(&self, id: AccountId) -> Result<Option<String>, StorageError> {
        // Check backend first
        if let Ok(Some(pw)) = self.backend.get_secret(id) {
            if !pw.is_empty() {
                return Ok(Some(pw));
            }
        }

        // Fallback to SQLite secure database
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT password FROM account_credentials WHERE account_id = ?1")?;
        let mut rows = stmt.query(params![id.to_string()])?;
        if let Some(row) = rows.next()? {
            let pw: String = row.get(0)?;
            // Best effort cache in backend
            let _ = self.backend.save_secret(id, &pw);
            return Ok(Some(pw));
        }

        Ok(None)
    }

    /// Deletes account credentials from both SQLite persistent storage and Keyring.
    pub fn delete_account_secret(&self, id: AccountId) -> Result<(), StorageError> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM account_credentials WHERE account_id = ?1",
                params![id.to_string()],
            )?;
        }
        let _ = self.backend.delete_secret(id);
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_audio_config(&self) -> Result<AudioConfig, StorageError> {
        if let Some(val) = self.get_setting("audio_config")? {
            if let Ok(cfg) = serde_json::from_str::<AudioConfig>(&val) {
                return Ok(cfg);
            }
        }
        Ok(AudioConfig::default())
    }

    pub fn save_audio_config(&self, config: &AudioConfig) -> Result<(), StorageError> {
        let val = serde_json::to_string(config)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        self.set_setting("audio_config", &val)
    }

    pub fn get_reject_mode(&self) -> Result<CallRejectMode, StorageError> {
        if let Some(val) = self.get_setting("reject_mode")? {
            return Ok(CallRejectMode::from_str_loose(&val));
        }
        Ok(CallRejectMode::default())
    }

    pub fn save_reject_mode(&self, mode: CallRejectMode) -> Result<(), StorageError> {
        self.set_setting("reject_mode", mode.as_str())
    }

    // -------------------------------------------------------------------------
    // Contact persistence & Caller ID Lookup
    // -------------------------------------------------------------------------

    pub fn save_contact(&self, contact: &Contact) -> Result<(), StorageError> {
        self.upsert_contacts(std::slice::from_ref(contact))
    }

    pub fn upsert_contacts(&self, contacts: &[Contact]) -> Result<(), StorageError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        for contact in contacts {
            tx.execute(
                r#"
                INSERT OR REPLACE INTO contacts (id, display_name, source, fritzbox_unique_id, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    contact.id.to_string(),
                    contact.display_name,
                    contact.source.to_string(),
                    contact.fritzbox_unique_id,
                    contact.updated_at.to_rfc3339(),
                ],
            )?;

            tx.execute(
                "DELETE FROM contact_numbers WHERE contact_id = ?1",
                params![contact.id.to_string()],
            )?;

            for num in &contact.numbers {
                let num_id = Uuid::new_v4().to_string();
                tx.execute(
                    r#"
                    INSERT INTO contact_numbers (id, contact_id, raw_number, normalized_digits, number_type, quickdial, vanity)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        num_id,
                        contact.id.to_string(),
                        num.raw_number,
                        num.normalized_digits,
                        num.number_type.to_string(),
                        num.quickdial,
                        num.vanity,
                    ],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn list_contacts(&self) -> Result<Vec<Contact>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, display_name, source, fritzbox_unique_id, updated_at FROM contacts ORDER BY display_name COLLATE NOCASE ASC"
        )?;

        let contact_rows = stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let id = Uuid::parse_str(&id_str)
                .map(ContactId)
                .unwrap_or_else(|_| ContactId::new());
            let display_name: String = row.get(1)?;
            let src_str: String = row.get(2)?;
            let source = if src_str == "fritzbox" {
                ContactSource::FritzBox
            } else {
                ContactSource::Local
            };
            let fritzbox_unique_id: Option<String> = row.get(3)?;
            let time_str: String = row.get(4)?;
            let updated_at = DateTime::parse_from_rfc3339(&time_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok((id, display_name, source, fritzbox_unique_id, updated_at))
        })?;

        let mut contacts = Vec::new();
        for res in contact_rows {
            let (id, display_name, source, fritzbox_unique_id, updated_at) = res?;

            let mut num_stmt = conn.prepare(
                "SELECT raw_number, normalized_digits, number_type, quickdial, vanity FROM contact_numbers WHERE contact_id = ?1"
            )?;
            let num_rows = num_stmt.query_map(params![id.to_string()], |nrow| {
                let raw_number: String = nrow.get(0)?;
                let normalized_digits: String = nrow.get(1)?;
                let ntype_str: String = nrow.get(2)?;
                let number_type = NumberType::from(ntype_str.as_str());
                let quickdial: Option<String> = nrow.get(3)?;
                let vanity: Option<String> = nrow.get(4)?;
                Ok(ContactNumber {
                    raw_number,
                    normalized_digits,
                    number_type,
                    quickdial,
                    vanity,
                })
            })?;

            let mut numbers = Vec::new();
            for n in num_rows {
                numbers.push(n?);
            }

            contacts.push(Contact {
                id,
                display_name,
                numbers,
                source,
                fritzbox_unique_id,
                updated_at,
            });
        }

        Ok(contacts)
    }

    pub fn search_contacts(&self, query: &str) -> Result<Vec<Contact>, StorageError> {
        let q = query.trim();
        if q.is_empty() {
            return self.list_contacts();
        }

        let all = self.list_contacts()?;
        let q_lower = q.to_lowercase();
        let is_numeric = q
            .chars()
            .all(|c| c.is_ascii_digit() || c == '*' || c == '#');

        let filtered: Vec<Contact> = all
            .into_iter()
            .filter(|c| {
                if c.display_name.to_lowercase().contains(&q_lower) {
                    return true;
                }
                for n in &c.numbers {
                    if n.raw_number.contains(q) || n.normalized_digits.contains(q) {
                        return true;
                    }
                    if let Some(ref qd) = n.quickdial {
                        if qd.contains(q) {
                            return true;
                        }
                    }
                }
                if is_numeric && q.len() >= 2 {
                    let name_t9 = text_to_t9(&c.display_name);
                    if name_t9.contains(q) {
                        return true;
                    }
                }
                false
            })
            .collect();

        Ok(filtered)
    }

    pub fn resolve_caller_name(&self, raw_number: &str) -> Result<Option<String>, StorageError> {
        let normalized = normalize_number_digits(raw_number);
        if normalized.is_empty() {
            return Ok(None);
        }

        let conn = self.conn.lock().unwrap();

        // 1. Exact match on normalized_digits or quickdial or raw_number
        let mut stmt = conn.prepare(
            r#"
            SELECT c.display_name
            FROM contacts c
            JOIN contact_numbers n ON c.id = n.contact_id
            WHERE n.normalized_digits = ?1 OR n.quickdial = ?1 OR n.raw_number = ?1
            LIMIT 1
            "#,
        )?;
        let mut rows = stmt.query(params![normalized])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(row.get(0)?));
        }

        // 2. Suffix match (e.g. matching +491729925904 with 01729925904)
        let pure_digits: String = normalized.chars().filter(|c| c.is_ascii_digit()).collect();
        if pure_digits.len() >= 6 {
            let suffix = if pure_digits.len() > 7 {
                &pure_digits[pure_digits.len() - 7..]
            } else {
                &pure_digits
            };
            let mut suffix_stmt = conn.prepare(
                r#"
                SELECT c.display_name
                FROM contacts c
                JOIN contact_numbers n ON c.id = n.contact_id
                WHERE n.normalized_digits LIKE '%' || ?1
                LIMIT 1
                "#,
            )?;
            let mut srows = suffix_stmt.query(params![suffix])?;
            if let Some(row) = srows.next()? {
                return Ok(Some(row.get(0)?));
            }
        }

        Ok(None)
    }

    pub fn delete_contact(&self, id: ContactId) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM contact_numbers WHERE contact_id = ?1",
            params![id.to_string()],
        )?;
        conn.execute(
            "DELETE FROM contacts WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn clear_fritzbox_contacts(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM contact_numbers WHERE contact_id IN (SELECT id FROM contacts WHERE source = 'fritzbox')",
            [],
        )?;
        conn.execute("DELETE FROM contacts WHERE source = 'fritzbox'", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_account_crud() {
        let storage = StorageManager::new_in_memory().expect("in-memory db");
        let acc = SipAccount {
            id: AccountId::new(),
            name: "FritzBox Test".into(),
            username: "620".into(),
            display_name: Some("Desk Phone".into()),
            domain: "fritz.box".into(),
            registrar: "fritz.box".into(),
            registrar_port: 5060,
            transport: TransportProtocol::Udp,
            expires_secs: 300,
            enabled: true,
            is_default: true,
        };

        storage.save_account(&acc).expect("save account");
        let list = storage.list_accounts().expect("list accounts");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].username, "620");

        let fetched = storage.get_account(acc.id).expect("get account");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().username, "620");

        storage.delete_account(acc.id).expect("delete account");
        let empty_list = storage.list_accounts().expect("list accounts");
        assert_eq!(empty_list.len(), 0);
        let fetched_after_delete = storage.get_account(acc.id).expect("get deleted account");
        assert!(fetched_after_delete.is_none());
    }

    #[test]
    fn test_audio_config_persistence() {
        let storage = StorageManager::new_in_memory().expect("in-memory db");
        let default_cfg = storage.get_audio_config().expect("default config");
        assert!(default_cfg.input_device_id.is_none());
        assert!(default_cfg.output_device_id.is_none());

        let mut custom_cfg = default_cfg;
        custom_cfg.input_device_id = Some("Jabra Evolve2 30 SE, USB Audio".into());
        custom_cfg.output_device_id = Some("Jabra Evolve2 30 SE, USB Audio".into());
        storage
            .save_audio_config(&custom_cfg)
            .expect("save audio config");

        let loaded = storage.get_audio_config().expect("load audio config");
        assert_eq!(
            loaded.input_device_id.as_deref(),
            Some("Jabra Evolve2 30 SE, USB Audio")
        );
        assert_eq!(
            loaded.output_device_id.as_deref(),
            Some("Jabra Evolve2 30 SE, USB Audio")
        );
    }

    #[test]
    fn test_account_credentials_persistence() {
        let storage = StorageManager::new_in_memory().expect("in-memory db");
        assert!(
            storage.is_isolated_backend(),
            "Test must use isolated in-memory credential backend"
        );
        let id = AccountId::new();

        assert_eq!(storage.get_account_secret(id).expect("get empty"), None);

        storage
            .save_account_secret(id, "super-secret-password")
            .expect("save secret");
        assert_eq!(
            storage.get_account_secret(id).expect("get secret"),
            Some("super-secret-password".to_string())
        );

        storage.delete_account_secret(id).expect("delete secret");
        assert_eq!(
            storage
                .get_account_secret(id)
                .expect("get empty after delete"),
            None
        );
    }

    #[test]
    fn test_isolated_backend_protection() {
        // 1. Verify two separate storage instances are completely isolated with independent in-memory state
        let storage1 = StorageManager::new_in_memory().expect("in-memory db 1");
        let storage2 = StorageManager::new_in_memory().expect("in-memory db 2");
        assert!(storage1.is_isolated_backend());
        assert!(storage2.is_isolated_backend());

        let id = AccountId::new();
        storage1
            .save_account_secret(id, "password-instance-1")
            .expect("save secret 1");

        assert_eq!(
            storage1.get_account_secret(id).expect("get secret 1"),
            Some("password-instance-1".to_string())
        );
        // Storage2 cannot see Storage1's credentials (no cross-instance interference)
        assert_eq!(storage2.get_account_secret(id).expect("get secret 2"), None);

        // 2. Verify KeyringCredentialBackend safety guard strictly prevents host keyring access during test execution
        let keyring_backend = KeyringCredentialBackend;
        assert!(!keyring_backend.is_isolated());
        let res = keyring_backend.save_secret(id, "unsafe-password");
        assert!(
            res.is_err(),
            "KeyringCredentialBackend must refuse host keyring operations during tests"
        );
    }

    #[test]
    fn test_contacts_persistence_search_and_lookup() {
        let storage = StorageManager::new_in_memory().expect("in-memory db");

        let mut c1 = Contact::new(
            "Alice Schmidt",
            ContactSource::FritzBox,
            Some("fb_001".into()),
        );
        c1.add_number(ContactNumber::new(
            "030 1234567",
            NumberType::Home,
            Some("**611".into()),
            None,
        ));
        c1.add_number(ContactNumber::new(
            "+49 172 9925904",
            NumberType::Mobile,
            None,
            None,
        ));

        let mut c2 = Contact::new("Bob Builder", ContactSource::Local, None);
        c2.add_number(ContactNumber::new(
            "0170 5551234",
            NumberType::Mobile,
            None,
            None,
        ));

        let mut c3 = Contact::new(
            "FRITZ!Box Service",
            ContactSource::FritzBox,
            Some("fb_002".into()),
        );
        c3.add_number(ContactNumber::new(
            "**9",
            NumberType::Internal,
            Some("**9".into()),
            None,
        ));

        storage
            .upsert_contacts(&[c1.clone(), c2.clone(), c3.clone()])
            .expect("upsert");

        // 1. List contacts
        let contacts = storage.list_contacts().expect("list");
        assert_eq!(contacts.len(), 3);
        assert_eq!(contacts[0].display_name, "Alice Schmidt");
        assert_eq!(contacts[0].numbers.len(), 2);
        assert_eq!(contacts[1].display_name, "Bob Builder");
        assert_eq!(contacts[2].display_name, "FRITZ!Box Service");

        // 2. Caller ID Resolution
        // Match exact normalized
        assert_eq!(
            storage
                .resolve_caller_name("+491729925904")
                .expect("resolve"),
            Some("Alice Schmidt".into())
        );
        // Match national format without country code against international
        assert_eq!(
            storage.resolve_caller_name("01729925904").expect("resolve"),
            Some("Alice Schmidt".into())
        );
        // Match internal quickdial
        assert_eq!(
            storage.resolve_caller_name("**611").expect("resolve"),
            Some("Alice Schmidt".into())
        );
        assert_eq!(
            storage.resolve_caller_name("**9").expect("resolve"),
            Some("FRITZ!Box Service".into())
        );
        // Non-existent
        assert_eq!(
            storage.resolve_caller_name("0999999").expect("resolve"),
            None
        );

        // 3. Search Contacts
        // Name query
        let res_name = storage.search_contacts("alice").expect("search name");
        assert_eq!(res_name.len(), 1);
        assert_eq!(res_name[0].display_name, "Alice Schmidt");

        // Number query
        let res_num = storage.search_contacts("5551234").expect("search num");
        assert_eq!(res_num.len(), 1);
        assert_eq!(res_num[0].display_name, "Bob Builder");

        // T9 Numeric search query (e.g. "254" for "ALI...")
        let res_t9 = storage.search_contacts("254").expect("search t9");
        assert_eq!(res_t9.len(), 1);
        assert_eq!(res_t9[0].display_name, "Alice Schmidt");

        // 4. Clear FRITZ!Box contacts
        storage.clear_fritzbox_contacts().expect("clear fb");
        let remaining = storage.list_contacts().expect("list remaining");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].display_name, "Bob Builder");
    }
}
