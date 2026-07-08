use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{docker, state};

pub fn list(root: &Path) -> Result<(), String> {
    let berths = state::list(root)?;

    if berths.is_empty() {
        println!("No active berths.");
        return Ok(());
    }

    let running = docker::running_projects().unwrap_or_default();

    println!(
        "{:<16} {:<16} {:<9} {:<6} PORTS",
        "NAME", "BRANCH", "STATUS", "AGE"
    );

    for berth in berths {
        let status = if running.contains(&berth.compose_project) {
            "running"
        } else {
            "stopped"
        };

        let mut ports: Vec<u16> = berth.ports.values().copied().collect();
        ports.sort_unstable();
        let ports = ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");

        println!(
            "{:<16} {:<16} {:<9} {:<6} {}",
            berth.name,
            berth.branch,
            status,
            format_age(berth.created_at),
            ports,
        );
    }

    Ok(())
}

/// Render the age of a berth from its creation timestamp as a compact string
/// (e.g. `3s`, `5m`, `2h`, `4d`). Returns `-` when the timestamp is missing.
pub fn format_age(created_at: u64) -> String {
    if created_at == 0 {
        return "-".to_string();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let secs = now.saturating_sub(created_at);

    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn missing_timestamp_renders_dash() {
        assert_eq!(format_age(0), "-");
    }

    #[test]
    fn recent_renders_seconds() {
        assert!(format_age(now().saturating_sub(5)).ends_with('s'));
    }

    #[test]
    fn units_scale_with_age() {
        assert_eq!(format_age(now().saturating_sub(120)), "2m");
        assert_eq!(format_age(now().saturating_sub(2 * 3600)), "2h");
        assert_eq!(format_age(now().saturating_sub(3 * 86400)), "3d");
    }
}
