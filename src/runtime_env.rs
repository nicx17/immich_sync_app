//! Provides best-effort checks of system conditions to determine when uploads should be deferred.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Returns true if the current primary network connection is metered.
pub fn is_metered_connection() -> bool {
    let output = match Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "GENERAL.METERED",
            "connection",
            "show",
            "--active",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };

    is_metered_connection_from_nmcli_output(&String::from_utf8_lossy(&output.stdout))
}

/// Returns true if the system appears to be running on battery power.
pub fn is_on_battery_power() -> bool {
    let power_supply_root = Path::new("/sys/class/power_supply");
    let entries = match fs::read_dir(power_supply_root) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    let mut statuses = Vec::new();

    for entry in entries.flatten() {
        let supply_path = entry.path();
        let supply_type = fs::read_to_string(supply_path.join("type"))
            .ok()
            .map(|value| value.trim().to_string());
        let online = fs::read_to_string(supply_path.join("online"))
            .ok()
            .map(|value| value.trim() == "1");
        let status = fs::read_to_string(supply_path.join("status"))
            .ok()
            .map(|value| value.trim().to_ascii_lowercase());
        statuses.push((supply_type, online, status));
    }

    is_on_battery_power_from_statuses(&statuses)
}

fn is_metered_connection_from_nmcli_output(stdout: &str) -> bool {
    let stdout = stdout.to_ascii_lowercase();
    stdout.contains("yes") || stdout.contains("guessed-yes")
}

fn is_on_battery_power_from_statuses(
    statuses: &[(Option<String>, Option<bool>, Option<String>)],
) -> bool {
    let mut found_battery = false;
    let mut mains_online = false;

    for (supply_type, online, status) in statuses {
        match supply_type.as_deref() {
            Some("Mains") | Some("USB") | Some("USB_C") => {
                if online.unwrap_or(false) {
                    mains_online = true;
                }
            }
            Some("Battery") => {
                found_battery = true;
                let status = status.as_deref().unwrap_or_default();
                if status == "charging" || status == "full" {
                    return false;
                }
            }
            _ => {}
        }
    }

    found_battery && !mains_online
}

#[cfg(test)]
mod tests {
    use super::{
        is_metered_connection, is_metered_connection_from_nmcli_output, is_on_battery_power,
        is_on_battery_power_from_statuses,
    };

    #[test]
    fn runtime_checks_are_safe() {
        let _ = is_metered_connection();
        let _ = is_on_battery_power();
    }

    #[test]
    fn test_metered_connection_parser() {
        assert!(is_metered_connection_from_nmcli_output(
            "GENERAL.METERED:yes\n"
        ));
        assert!(is_metered_connection_from_nmcli_output(
            "GENERAL.METERED:guessed-yes\n"
        ));
        assert!(!is_metered_connection_from_nmcli_output(
            "GENERAL.METERED:no\n"
        ));
    }

    #[test]
    fn test_battery_power_detection_from_statuses() {
        assert!(is_on_battery_power_from_statuses(&[
            (Some("Battery".into()), None, Some("discharging".into())),
            (Some("Mains".into()), Some(false), None),
        ]));

        assert!(!is_on_battery_power_from_statuses(&[
            (Some("Battery".into()), None, Some("charging".into())),
            (Some("Mains".into()), Some(true), None),
        ]));

        assert!(!is_on_battery_power_from_statuses(&[(
            Some("Mains".into()),
            Some(true),
            None,
        )]));
    }
}
