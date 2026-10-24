//! KDE Plasma StatusNotifierItem (System Tray) & DBusMenu Integration.
//!
//! Provides:
//! - `org.kde.StatusNotifierItem`: Freedesktop / KDE system tray protocol
//!   - Left-click (`Activate`) to toggle / restore softphone window
//!   - `ProvideXdgActivationToken` for smooth Wayland window activation
//!   - Visual indication (status switches to "NeedsAttention", pulses call-start icon on incoming call)
//!   - Rich tooltip with registration and call status
//! - `com.canonical.dbusmenu`: Native context menu on right-click
//!   - "Open Plasma SIP"
//!   - Last 3 incoming calls (clickable to dial)
//!   - Last 3 outgoing calls (clickable to dial)
//!   - "Quit Plasma SIP"

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use zbus::interface;
use zbus::zvariant::Value;

pub type MenuPropMap = HashMap<String, Value<'static>>;
pub type MenuItemTuple = (i32, MenuPropMap, Vec<Value<'static>>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayCallStatus {
    Idle,
    IncomingRinging { caller: String },
    ActiveCall { remote: String },
}

#[derive(Clone, Debug)]
pub enum TrayAction {
    ToggleWindow,
    ShowWindow,
    Dial(String),
    PrefillAndShow(String),
    AnswerCall,
    HangupCall,
    Quit,
}

#[derive(Clone)]
pub struct StatusNotifierItemService {
    pub status: Arc<RwLock<TrayCallStatus>>,
    pub registered: Arc<RwLock<bool>>,
    pub action_tx: tokio::sync::mpsc::UnboundedSender<TrayAction>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItemService {
    #[zbus(property)]
    async fn category(&self) -> String {
        "Communications".to_string()
    }

    #[zbus(property)]
    async fn id(&self) -> String {
        "plasma-sip".to_string()
    }

    #[zbus(property)]
    async fn title(&self) -> String {
        "Plasma SIP".to_string()
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        let call_st = self.status.read().await;
        match *call_st {
            TrayCallStatus::IncomingRinging { .. } => "NeedsAttention".to_string(),
            _ => "Active".to_string(),
        }
    }

    #[zbus(property)]
    async fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    async fn icon_name(&self) -> String {
        "call-start".to_string()
    }

    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }

    #[zbus(property)]
    async fn overlay_icon_name(&self) -> String {
        "".to_string()
    }

    #[zbus(property)]
    async fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }

    #[zbus(property)]
    async fn attention_icon_name(&self) -> String {
        "call-start".to_string()
    }

    #[zbus(property)]
    async fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }

    #[zbus(property)]
    async fn attention_movie_name(&self) -> String {
        "".to_string()
    }

    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        let call_st = self.status.read().await;
        let desc = match &*call_st {
            TrayCallStatus::IncomingRinging { caller } => format!("Incoming Call: {}", caller),
            TrayCallStatus::ActiveCall { remote } => format!("Active Call: {}", remote),
            TrayCallStatus::Idle => {
                if *self.registered.read().await {
                    "Registered & Ready".to_string()
                } else {
                    "Plasma SIP Softphone".to_string()
                }
            }
        };
        (
            "call-start".to_string(),
            vec![],
            "Plasma SIP".to_string(),
            desc,
        )
    }

    #[zbus(property)]
    async fn icon_theme_path(&self) -> String {
        "".to_string()
    }

    #[zbus(property)]
    async fn menu(&self) -> zbus::zvariant::ObjectPath<'_> {
        zbus::zvariant::ObjectPath::try_from("/MenuBar").unwrap()
    }

    #[zbus(property)]
    async fn item_is_menu(&self) -> bool {
        false
    }

    async fn activate(&self, _x: i32, _y: i32) {
        info!("StatusNotifierItem Activate triggered by user");
        let _ = self.action_tx.send(TrayAction::ToggleWindow);
    }

    async fn secondary_activate(&self, _x: i32, _y: i32) {
        let call_st = self.status.read().await;
        match *call_st {
            TrayCallStatus::IncomingRinging { .. } => {
                let _ = self.action_tx.send(TrayAction::AnswerCall);
            }
            _ => {
                let _ = self.action_tx.send(TrayAction::ToggleWindow);
            }
        }
    }

    async fn context_menu(&self, _x: i32, _y: i32) {
        // Fallback for environments without native DBusMenu
        let _ = self.action_tx.send(TrayAction::ToggleWindow);
    }

    async fn scroll(&self, _delta: i32, _orientation: &str) {}

    async fn provide_xdg_activation_token(&self, token: &str) {
        info!(token = %token, "Received XDG activation token from KDE Plasma");
        // SAFETY: Setting environment variable provided by KDE Plasma compositor.
        unsafe {
            std::env::set_var("XDG_ACTIVATION_TOKEN", token);
        }
    }

    #[zbus(signal)]
    pub async fn new_status(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        status: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_tool_tip(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_icon(signal_ctxt: &zbus::object_server::SignalEmitter<'_>)
        -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_attention_icon(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
}

/// Implementation of the `com.canonical.dbusmenu` interface for native KDE Plasma right-click menu.
#[derive(Clone)]
pub struct DBusMenuService {
    pub recent_incoming: Arc<RwLock<Vec<(String, String)>>>,
    pub recent_outgoing: Arc<RwLock<Vec<(String, String)>>>,
    pub revision: Arc<AtomicU32>,
    pub action_tx: tokio::sync::mpsc::UnboundedSender<TrayAction>,
}

#[interface(name = "com.canonical.dbusmenu")]
impl DBusMenuService {
    #[zbus(property)]
    async fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        "normal".to_string()
    }

    async fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, MenuItemTuple) {
        let rev = self.revision.load(Ordering::SeqCst);
        let in_calls = self.recent_incoming.read().await;
        let out_calls = self.recent_outgoing.read().await;

        let mut children = Vec::new();

        // 1. Open Plasma SIP
        children.push(create_menu_item(
            1,
            "Open Plasma SIP",
            Some("call-start"),
            true,
        ));
        // 2. Separator
        children.push(create_separator(2));

        // 3. Section: Recent Incoming Calls
        children.push(create_section_header(3, "Recent Incoming Calls:"));
        if in_calls.is_empty() {
            children.push(create_disabled_item(10, "  (No recent incoming calls)"));
        } else {
            for (i, (label, _target)) in in_calls.iter().take(3).enumerate() {
                children.push(create_menu_item(
                    10 + i as i32,
                    &format!("  📞 {}", label),
                    Some("call-start"),
                    true,
                ));
            }
        }

        // 20. Separator
        children.push(create_separator(20));

        // 21. Section: Recent Outgoing Calls
        children.push(create_section_header(21, "Recent Outgoing Calls:"));
        if out_calls.is_empty() {
            children.push(create_disabled_item(30, "  (No recent outgoing calls)"));
        } else {
            for (j, (label, _target)) in out_calls.iter().take(3).enumerate() {
                children.push(create_menu_item(
                    30 + j as i32,
                    &format!("  📞 {}", label),
                    Some("call-start"),
                    true,
                ));
            }
        }

        // 90. Separator
        children.push(create_separator(90));

        // 99. Quit Plasma SIP
        children.push(create_menu_item(
            99,
            "Quit Plasma SIP",
            Some("application-exit"),
            true,
        ));

        let mut root_props = HashMap::new();
        root_props.insert(
            "children-display".to_string(),
            Value::from("submenu".to_string()),
        );

        (rev, (0, root_props, children))
    }

    async fn get_group_properties(
        &self,
        _ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, MenuPropMap)> {
        vec![]
    }

    async fn get_property(&self, _id: i32, _name: &str) -> Value<'_> {
        Value::from("")
    }

    async fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id == "clicked" {
            info!(item_id = id, "Tray menu item clicked");
            if id == 1 {
                let _ = self.action_tx.send(TrayAction::ShowWindow);
            } else if (10..20).contains(&id) {
                let idx = (id - 10) as usize;
                let in_calls = self.recent_incoming.read().await;
                if let Some((_, target)) = in_calls.get(idx) {
                    let _ = self.action_tx.send(TrayAction::Dial(target.clone()));
                }
            } else if (30..40).contains(&id) {
                let idx = (id - 30) as usize;
                let out_calls = self.recent_outgoing.read().await;
                if let Some((_, target)) = out_calls.get(idx) {
                    let _ = self.action_tx.send(TrayAction::Dial(target.clone()));
                }
            } else if id == 99 {
                let _ = self.action_tx.send(TrayAction::Quit);
            }
        }
    }

    async fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    #[zbus(signal)]
    pub async fn layout_updated(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

/// D-Bus interface for Single-Instance activation and desktop integration
#[derive(Clone)]
pub struct SingleInstanceService {
    pub action_tx: tokio::sync::mpsc::UnboundedSender<TrayAction>,
}

#[interface(name = "org.kde.plasma_sip.ui")]
impl SingleInstanceService {
    async fn activate(&self) {
        info!("Received D-Bus Activate request from secondary instance");
        let _ = self.action_tx.send(TrayAction::ShowWindow);
    }

    async fn dial(&self, target: &str) {
        info!("Received D-Bus Dial request from secondary instance");
        let (_, num) = parse_caller_info(target);
        let dial_tgt = if num.is_empty() {
            target.to_string()
        } else {
            num
        };
        // Security (SEC-01): Populate dialer and require user confirmation before placing call.
        let _ = self.action_tx.send(TrayAction::PrefillAndShow(dial_tgt));
    }
}

fn create_menu_item(
    id: i32,
    label: &str,
    icon_name: Option<&str>,
    enabled: bool,
) -> Value<'static> {
    let mut props = HashMap::new();
    props.insert("label".to_string(), Value::from(label.to_string()));
    props.insert("enabled".to_string(), Value::from(enabled));
    props.insert("visible".to_string(), Value::from(true));
    if let Some(icon) = icon_name {
        props.insert("icon-name".to_string(), Value::from(icon.to_string()));
    }
    Value::from((id, props, Vec::<Value<'static>>::new()))
}

fn create_separator(id: i32) -> Value<'static> {
    let mut props = HashMap::new();
    props.insert("type".to_string(), Value::from("separator".to_string()));
    props.insert("visible".to_string(), Value::from(true));
    Value::from((id, props, Vec::<Value<'static>>::new()))
}

fn create_section_header(id: i32, label: &str) -> Value<'static> {
    let mut props = HashMap::new();
    props.insert("label".to_string(), Value::from(label.to_string()));
    props.insert("enabled".to_string(), Value::from(false));
    props.insert("visible".to_string(), Value::from(true));
    Value::from((id, props, Vec::<Value<'static>>::new()))
}

fn create_disabled_item(id: i32, label: &str) -> Value<'static> {
    let mut props = HashMap::new();
    props.insert("label".to_string(), Value::from(label.to_string()));
    props.insert("enabled".to_string(), Value::from(false));
    props.insert("visible".to_string(), Value::from(true));
    Value::from((id, props, Vec::<Value<'static>>::new()))
}

pub fn parse_caller_info(raw: &str) -> (String, String) {
    let s = raw.trim();
    if s.is_empty() {
        return (String::new(), String::new());
    }

    // 1. Extract Display Name (quoted "..." or unquoted before '<')
    let (display_name, uri_part) = if let Some(q_start) = s.find('"') {
        if let Some(q_end) = s[q_start + 1..].find('"') {
            let name = s[q_start + 1..q_start + 1 + q_end].trim();
            let rest = s[q_start + 1 + q_end + 1..].trim();
            (
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                },
                rest,
            )
        } else {
            (None, s)
        }
    } else if let Some(angle_start) = s.find('<') {
        let name = s[..angle_start].trim();
        let rest = &s[angle_start..];
        (
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            },
            rest,
        )
    } else {
        (None, s)
    };

    // 2. Extract target number/user from uri_part
    // Handle <...>, sip:..., sips:..., tel:..., or plain number/user
    let inner = if let (Some(start), Some(end)) = (uri_part.find('<'), uri_part.rfind('>')) {
        if start < end {
            &uri_part[start + 1..end]
        } else {
            uri_part
        }
    } else {
        uri_part
    }
    .trim();

    let after_scheme = if let Some(sips_idx) = inner.find("sips:") {
        &inner[sips_idx + 5..]
    } else if let Some(sip_idx) = inner.find("sip:") {
        &inner[sip_idx + 4..]
    } else if let Some(tel_idx) = inner.find("tel:") {
        &inner[tel_idx + 4..]
    } else {
        inner
    };

    let end_idx = after_scheme
        .find(['@', ';', '?', '>', ' '])
        .unwrap_or(after_scheme.len());
    let number = after_scheme[..end_idx].trim().to_string();

    let label = match display_name {
        Some(ref name) if !name.is_empty() && name != &number => {
            format!("{} ({})", name, number)
        }
        _ => number.clone(),
    };

    (label, number)
}

#[zbus::proxy(
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher",
    interface = "org.kde.StatusNotifierWatcher"
)]
pub trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

pub struct TrayHandle {
    pub status: Arc<RwLock<TrayCallStatus>>,
    pub registered: Arc<RwLock<bool>>,
    pub recent_incoming: Arc<RwLock<Vec<(String, String)>>>,
    pub recent_outgoing: Arc<RwLock<Vec<(String, String)>>>,
    pub menu_revision: Arc<AtomicU32>,
    pub connection: zbus::Connection,
}

#[derive(serde::Deserialize)]
struct HistoryEntry {
    direction: String,
    remote_party: String,
}

impl TrayHandle {
    pub async fn set_status(&self, new_st: TrayCallStatus) {
        {
            let mut st = self.status.write().await;
            *st = new_st.clone();
        }
        if let Ok(emitter) =
            zbus::object_server::SignalEmitter::new(&self.connection, "/StatusNotifierItem")
        {
            let status_str = match new_st {
                TrayCallStatus::IncomingRinging { .. } => "NeedsAttention",
                _ => "Active",
            };
            let _ = StatusNotifierItemService::new_status(&emitter, status_str).await;
            if matches!(new_st, TrayCallStatus::IncomingRinging { .. }) {
                let _ = StatusNotifierItemService::new_attention_icon(&emitter).await;
            }
            let _ = StatusNotifierItemService::new_tool_tip(&emitter).await;
        }
    }

    pub async fn set_registered(&self, is_reg: bool) {
        {
            let mut reg = self.registered.write().await;
            *reg = is_reg;
        }
        if let Ok(emitter) =
            zbus::object_server::SignalEmitter::new(&self.connection, "/StatusNotifierItem")
        {
            let _ = StatusNotifierItemService::new_tool_tip(&emitter).await;
        }
    }

    pub async fn update_call_history(&self, history_json: &str) {
        if let Ok(entries) = serde_json::from_str::<Vec<HistoryEntry>>(history_json) {
            let mut in_list = Vec::new();
            let mut out_list = Vec::new();

            for entry in entries {
                let (label, target) = parse_caller_info(&entry.remote_party);
                if target.is_empty() {
                    continue;
                }
                if entry.direction.eq_ignore_ascii_case("inbound") {
                    if in_list.len() < 3 && !in_list.iter().any(|(_, t)| t == &target) {
                        in_list.push((label, target));
                    }
                } else if entry.direction.eq_ignore_ascii_case("outbound")
                    && out_list.len() < 3
                    && !out_list.iter().any(|(_, t)| t == &target)
                {
                    out_list.push((label, target));
                }
            }

            {
                let mut inc = self.recent_incoming.write().await;
                *inc = in_list;
            }
            {
                let mut outc = self.recent_outgoing.write().await;
                *outc = out_list;
            }

            let rev = self.menu_revision.fetch_add(1, Ordering::SeqCst) + 1;
            if let Ok(emitter) =
                zbus::object_server::SignalEmitter::new(&self.connection, "/MenuBar")
            {
                let _ = DBusMenuService::layout_updated(&emitter, rev, 0).await;
            }
        }
    }
}

pub async fn setup_tray(
    conn: &zbus::Connection,
    action_tx: tokio::sync::mpsc::UnboundedSender<TrayAction>,
) -> Option<TrayHandle> {
    let status = Arc::new(RwLock::new(TrayCallStatus::Idle));
    let registered = Arc::new(RwLock::new(false));
    let recent_incoming = Arc::new(RwLock::new(Vec::new()));
    let recent_outgoing = Arc::new(RwLock::new(Vec::new()));
    let menu_revision = Arc::new(AtomicU32::new(1));

    let item = StatusNotifierItemService {
        status: status.clone(),
        registered: registered.clone(),
        action_tx: action_tx.clone(),
    };

    let menu = DBusMenuService {
        recent_incoming: recent_incoming.clone(),
        recent_outgoing: recent_outgoing.clone(),
        revision: menu_revision.clone(),
        action_tx: action_tx.clone(),
    };

    let single_instance = SingleInstanceService {
        action_tx: action_tx.clone(),
    };

    if let Err(e) = conn.object_server().at("/StatusNotifierItem", item).await {
        warn!(error = %e, "Failed to register /StatusNotifierItem on D-Bus connection");
        return None;
    }

    if let Err(e) = conn.object_server().at("/MenuBar", menu).await {
        warn!(error = %e, "Failed to register /MenuBar (DBusMenu) on D-Bus connection");
    } else {
        info!("Registered /MenuBar DBusMenu service for tray context menu");
    }

    if let Err(e) = conn
        .object_server()
        .at("/org/kde/plasma_sip/ui", single_instance)
        .await
    {
        warn!(error = %e, "Failed to register /org/kde/plasma_sip/ui");
    } else {
        match conn.request_name("org.kde.plasma_sip.ui").await {
            Ok(reply) => info!(?reply, "Registered /org/kde/plasma_sip/ui single-instance IPC service"),
            Err(e) => warn!(error = %e, "Failed to request D-Bus name org.kde.plasma_sip.ui"),
        }
    }

    match StatusNotifierWatcherProxy::new(conn).await {
        Ok(watcher) => {
            if let Err(e) = watcher
                .register_status_notifier_item("/StatusNotifierItem")
                .await
            {
                warn!(error = %e, "Failed to register StatusNotifierItem with org.kde.StatusNotifierWatcher");
            } else {
                info!("Registered StatusNotifierItem with KDE Plasma StatusNotifierWatcher");
            }
        }
        Err(e) => {
            info!(error = %e, "StatusNotifierWatcher not available (non-KDE or headless session)");
        }
    }

    Some(TrayHandle {
        status,
        registered,
        recent_incoming,
        recent_outgoing,
        menu_revision,
        connection: conn.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_caller_info_formats() {
        // Quoted name with SIP URI (using verified BNetzA drama number)
        let (label, target) = parse_caller_info("\"Alice\" <sip:01729925904@fritz.box>");
        assert_eq!(label, "Alice (01729925904)");
        assert_eq!(target, "01729925904");

        // Quoted name with SIPS URI
        let (label, target) = parse_caller_info("\"Alice\" <sips:01729925904@fritz.box>");
        assert_eq!(label, "Alice (01729925904)");
        assert_eq!(target, "01729925904");

        // Unquoted name with angle brackets
        let (label, target) = parse_caller_info("Office PBX <sip:200@fritz.box>");
        assert_eq!(label, "Office PBX (200)");
        assert_eq!(target, "200");

        // Bracketed number without name
        let (label, target) = parse_caller_info("<sip:01729925904@fritz.box>");
        assert_eq!(label, "01729925904");
        assert_eq!(target, "01729925904");

        // Raw SIP URI
        let (label, target) = parse_caller_info("sip:01729925904@fritz.box");
        assert_eq!(label, "01729925904");
        assert_eq!(target, "01729925904");

        // Tel URI
        let (label, target) = parse_caller_info("\"Emergency\" <tel:112>");
        assert_eq!(label, "Emergency (112)");
        assert_eq!(target, "112");

        // Plain dialed number
        let (label, target) = parse_caller_info("01729925904");
        assert_eq!(label, "01729925904");
        assert_eq!(target, "01729925904");

        // Angle brackets with plain number
        let (label, target) = parse_caller_info("<01729925904>");
        assert_eq!(label, "01729925904");
        assert_eq!(target, "01729925904");

        // Empty string
        let (label, target) = parse_caller_info("");
        assert_eq!(label, "");
        assert_eq!(target, "");
    }
}
