//! Core service orchestration and D-Bus IPC service for Plasma SIP.

pub mod account_manager;
pub mod call_manager;
pub mod dbus_api;

pub use account_manager::*;
pub use call_manager::*;
pub use dbus_api::*;
