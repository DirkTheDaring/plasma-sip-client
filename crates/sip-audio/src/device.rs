use crate::error::AudioError;
use cpal::traits::{DeviceTrait, HostTrait};
use sip_domain::{AudioDeviceInfo, DomainError};
use tracing::warn;

pub fn list_input_devices() -> Result<Vec<AudioDeviceInfo>, DomainError> {
    let host = cpal::default_host();
    let default_device_name = host
        .default_input_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()));

    let mut devices = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Ok(dev_iter) = host.input_devices() {
        for dev in dev_iter {
            if let Ok(desc) = dev.description() {
                let name = desc.name().to_string();
                if name.starts_with("Discard all samples") {
                    continue;
                }
                if seen.insert(name.clone()) {
                    let is_default = default_device_name.as_ref() == Some(&name);
                    devices.push(AudioDeviceInfo {
                        id: name.clone(),
                        name,
                        is_default,
                    });
                }
            }
        }
    }
    Ok(devices)
}

pub fn list_output_devices() -> Result<Vec<AudioDeviceInfo>, DomainError> {
    let host = cpal::default_host();
    let default_device_name = host
        .default_output_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()));

    let mut devices = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Ok(dev_iter) = host.output_devices() {
        for dev in dev_iter {
            if let Ok(desc) = dev.description() {
                let name = desc.name().to_string();
                if name.starts_with("Discard all samples") {
                    continue;
                }
                if seen.insert(name.clone()) {
                    let is_default = default_device_name.as_ref() == Some(&name);
                    devices.push(AudioDeviceInfo {
                        id: name.clone(),
                        name,
                        is_default,
                    });
                }
            }
        }
    }
    Ok(devices)
}

/// Finds the requested input device by ID or name, falling back to the default input device.
pub fn find_input_device(id_or_name: Option<&str>) -> Result<cpal::Device, AudioError> {
    let host = cpal::default_host();

    if let Some(target) = id_or_name {
        let trimmed = target.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("default") {
            if let Ok(devices) = host.input_devices() {
                for dev in devices {
                    if let Ok(desc) = dev.description() {
                        let name = desc.name();
                        if name == trimmed || name.contains(trimmed) {
                            return Ok(dev);
                        }
                    }
                }
            }
            warn!(
                "Requested input device '{}' not found, falling back to default",
                trimmed
            );
        }
    }

    host.default_input_device()
        .ok_or(AudioError::NoDeviceAvailable)
}

/// Finds the requested output device by ID or name, falling back to the default output device.
pub fn find_output_device(id_or_name: Option<&str>) -> Result<cpal::Device, AudioError> {
    let host = cpal::default_host();

    if let Some(target) = id_or_name {
        let trimmed = target.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("default") {
            if let Ok(devices) = host.output_devices() {
                for dev in devices {
                    if let Ok(desc) = dev.description() {
                        let name = desc.name();
                        if name == trimmed || name.contains(trimmed) {
                            return Ok(dev);
                        }
                    }
                }
            }
            warn!(
                "Requested output device '{}' not found, falling back to default",
                trimmed
            );
        }
    }

    host.default_output_device()
        .ok_or(AudioError::NoDeviceAvailable)
}
