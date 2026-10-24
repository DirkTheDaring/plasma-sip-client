//! Platform integration for KDE Plasma (Freedesktop Notifications, systemd user services).

use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::info;
use zbus::Connection;

use sip_domain::CallId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAction {
    Answer,
    Reject,
    Open,
}

#[zbus::proxy(
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    interface = "org.freedesktop.Notifications"
)]
pub trait FreedesktopNotifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: &HashMap<&str, zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct PlatformNotifier {
    connection: Option<Connection>,
    active_notifications: Arc<Mutex<HashMap<u32, CallId>>>,
    action_tx: broadcast::Sender<(CallId, NotificationAction)>,
}

impl PlatformNotifier {
    fn spawn_action_listener(
        conn: &Connection,
        active_notifications: Arc<Mutex<HashMap<u32, CallId>>>,
        action_tx: broadcast::Sender<(CallId, NotificationAction)>,
    ) {
        let conn_clone = conn.clone();
        tokio::spawn(async move {
            if let Ok(proxy) = FreedesktopNotificationsProxy::new(&conn_clone).await {
                if let Ok(mut stream) = proxy.receive_action_invoked().await {
                    while let Some(signal) = stream.next().await {
                        if let Ok(args) = signal.args() {
                            let notif_id = args.id;
                            let action_key = args.action_key;
                            let call_id = {
                                let mut map = active_notifications.lock().await;
                                map.remove(&notif_id)
                            };
                            if let Some(cid) = call_id {
                                let act = match action_key {
                                    "answer" => Some(NotificationAction::Answer),
                                    "reject" => Some(NotificationAction::Reject),
                                    "default" | "open" => Some(NotificationAction::Open),
                                    _ => None,
                                };
                                if let Some(a) = act {
                                    let _ = action_tx.send((cid, a));
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    pub async fn new() -> Self {
        let connection = Connection::session().await.ok();
        Self::with_connection(connection)
    }

    pub fn with_connection(connection: Option<Connection>) -> Self {
        let (action_tx, _) = broadcast::channel(32);
        let active_notifications = Arc::new(Mutex::new(HashMap::new()));

        if let Some(ref conn) = connection {
            Self::spawn_action_listener(conn, active_notifications.clone(), action_tx.clone());
        }

        Self {
            connection,
            active_notifications,
            action_tx,
        }
    }

    pub fn subscribe_actions(&self) -> broadcast::Receiver<(CallId, NotificationAction)> {
        self.action_tx.subscribe()
    }

    /// Triggers or simulates a notification action (e.g. Answer or Reject)
    pub fn trigger_action(&self, call_id: CallId, action: NotificationAction) {
        let _ = self.action_tx.send((call_id, action));
    }

    /// Emits a desktop notification for an incoming call with Answer/Reject actions
    pub async fn notify_incoming_call(
        &self,
        call_id: CallId,
        caller_id: &str,
        caller_name: Option<&str>,
        called_line: Option<&str>,
    ) -> Result<u32, String> {
        let conn = match self.connection.as_ref() {
            Some(c) => c,
            None => {
                info!(call_id = %call_id, "Desktop notification skipped (no D-Bus session connection)");
                return Ok(0);
            }
        };

        let title = format!("Incoming Call: {}", caller_id);
        let main_desc = if let Some(name) = caller_name {
            if name.eq_ignore_ascii_case(caller_id) {
                caller_id.to_string()
            } else {
                format!("{} ({})", name, caller_id)
            }
        } else {
            caller_id.to_string()
        };

        let body = if let Some(line) = called_line {
            if !line.is_empty() {
                format!("{}\nLine: {}", main_desc, line)
            } else {
                main_desc
            }
        } else {
            main_desc
        };

        let actions: Vec<&str> = vec!["open", "Open Call", "answer", "Answer", "reject", "Decline"];
        let mut hints: HashMap<&str, zvariant::Value<'_>> = HashMap::new();
        hints.insert("urgency", zvariant::Value::from(2u8));
        hints.insert("category", zvariant::Value::from("call.incoming"));
        hints.insert("sound-name", zvariant::Value::from("phone-incoming-call"));
        hints.insert("desktop-entry", zvariant::Value::from("org.kde.plasma_sip"));

        let proxy = match FreedesktopNotificationsProxy::new(conn).await {
            Ok(p) => p,
            Err(e) => {
                info!(error = %e, "Desktop notification server not available");
                return Ok(0);
            }
        };

        match proxy
            .notify(
                "Plasma SIP",
                0u32,
                "call-start",
                &title,
                &body,
                &actions,
                &hints,
                -1i32,
            )
            .await
        {
            Ok(reply) => {
                self.active_notifications
                    .lock()
                    .await
                    .insert(reply, call_id);
                info!(
                    notification_id = reply,
                    call_id = %call_id,
                    "Posted incoming call desktop notification"
                );
                Ok(reply)
            }
            Err(e) => {
                info!(error = %e, "Failed to post desktop notification (fallback to silent)");
                Ok(0)
            }
        }
    }

    /// Closes an active notification by notification ID
    pub async fn close_notification(&self, id: u32) -> Result<(), String> {
        if id == 0 {
            return Ok(());
        }
        self.active_notifications.lock().await.remove(&id);
        if let Some(ref conn) = self.connection {
            if let Ok(proxy) = FreedesktopNotificationsProxy::new(conn).await {
                let _ = proxy.close_notification(id).await;
            }
        }
        Ok(())
    }

    /// Closes any active notification associated with the given CallId
    pub async fn close_call_notification(&self, call_id: CallId) -> Result<(), String> {
        let id_to_close = {
            let mut map = self.active_notifications.lock().await;
            let found = map
                .iter()
                .find(|&(_, cid)| *cid == call_id)
                .map(|(&id, _)| id);
            if let Some(id) = found {
                map.remove(&id);
                Some(id)
            } else {
                None
            }
        };

        if let Some(id) = id_to_close {
            if id != 0 {
                if let Some(ref conn) = self.connection {
                    if let Ok(proxy) = FreedesktopNotificationsProxy::new(conn).await {
                        let _ = proxy.close_notification(id).await;
                    }
                }
            }
        }
        Ok(())
    }
}
