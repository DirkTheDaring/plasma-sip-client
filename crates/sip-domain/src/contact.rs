use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContactId(pub Uuid);

impl ContactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ContactId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ContactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NumberType {
    #[default]
    Home,
    Mobile,
    Work,
    Internal,
    Fax,
    Other,
}

impl fmt::Display for NumberType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Home => write!(f, "Home"),
            Self::Mobile => write!(f, "Mobile"),
            Self::Work => write!(f, "Work"),
            Self::Internal => write!(f, "Internal"),
            Self::Fax => write!(f, "Fax"),
            Self::Other => write!(f, "Other"),
        }
    }
}

impl From<&str> for NumberType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "home" => Self::Home,
            "mobile" => Self::Mobile,
            "work" => Self::Work,
            "internal" => Self::Internal,
            "fax_work" | "fax_home" | "fax" => Self::Fax,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContactSource {
    #[default]
    Local,
    FritzBox,
}

impl fmt::Display for ContactSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::FritzBox => write!(f, "fritzbox"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactNumber {
    pub raw_number: String,
    pub normalized_digits: String,
    pub number_type: NumberType,
    pub quickdial: Option<String>,
    pub vanity: Option<String>,
}

impl ContactNumber {
    pub fn new(
        raw_number: impl Into<String>,
        number_type: NumberType,
        quickdial: Option<String>,
        vanity: Option<String>,
    ) -> Self {
        let raw = raw_number.into();
        let normalized = normalize_number_digits(&raw);
        Self {
            raw_number: raw,
            normalized_digits: normalized,
            number_type,
            quickdial,
            vanity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: ContactId,
    pub display_name: String,
    pub numbers: Vec<ContactNumber>,
    pub source: ContactSource,
    pub fritzbox_unique_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl Contact {
    pub fn new(
        display_name: impl Into<String>,
        source: ContactSource,
        fritzbox_unique_id: Option<String>,
    ) -> Self {
        Self {
            id: ContactId::new(),
            display_name: display_name.into(),
            numbers: Vec::new(),
            source,
            fritzbox_unique_id,
            updated_at: Utc::now(),
        }
    }

    pub fn add_number(&mut self, number: ContactNumber) {
        self.numbers.push(number);
    }
}

/// Normalizes phone numbers to standard searchable digits.
/// Preserves internal PBX extensions like `**610` or `**9`.
/// Replaces `00` international prefix with `+`.
pub fn normalize_number_digits(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Preserve FRITZ!Box internal extensions
    if trimmed.starts_with("**") {
        let ext_digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        return format!("**{}", ext_digits);
    }

    // Extract digits and '+'
    let mut digits: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();

    // Replace leading 00 with +
    if digits.starts_with("00") {
        digits = format!("+{}", &digits[2..]);
    }

    digits
}

/// Converts alphabetical characters to phone keypad T9 digits (2-9).
/// Allows search queries like 'ALI' (254) to match names.
pub fn char_to_t9(c: char) -> Option<char> {
    match c.to_ascii_lowercase() {
        'a' | 'b' | 'c' => Some('2'),
        'd' | 'e' | 'f' => Some('3'),
        'g' | 'h' | 'i' => Some('4'),
        'j' | 'k' | 'l' => Some('5'),
        'm' | 'n' | 'o' => Some('6'),
        'p' | 'q' | 'r' | 's' => Some('7'),
        't' | 'u' | 'v' => Some('8'),
        'w' | 'x' | 'y' | 'z' => Some('9'),
        _ => None,
    }
}

/// Converts a text string into its T9 numeric equivalent.
pub fn text_to_t9(text: &str) -> String {
    text.chars().filter_map(char_to_t9).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_normalization() {
        assert_eq!(normalize_number_digits("030 / 1234567"), "0301234567");
        assert_eq!(
            normalize_number_digits("+49 (0) 172 992 5904"),
            "+4901729925904"
        );
        assert_eq!(normalize_number_digits("0049 30 12345"), "+493012345");
        assert_eq!(normalize_number_digits("**610"), "**610");
        assert_eq!(normalize_number_digits("**9"), "**9");
    }

    #[test]
    fn test_t9_conversion() {
        assert_eq!(text_to_t9("Alice"), "25423");
        assert_eq!(text_to_t9("Bob"), "262");
        assert_eq!(text_to_t9("Fritz!Box"), "37489269");
    }
}
