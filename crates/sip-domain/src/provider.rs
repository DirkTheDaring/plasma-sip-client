use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    FritzBox,
    GenericSip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialTarget {
    pub raw: String,
    pub normalized_sip_uri: String,
    pub display_name: Option<String>,
    pub is_internal_extension: bool,
}
