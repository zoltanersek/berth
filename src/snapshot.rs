use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::{docker, ls, state, validate};

const SNAPSHOTS_DIR: &str = ".berth/snapshots";

#[derive(Subcommand, Debug, Clone)]
pub enum SnapshotCmd {
    /// Capture a berth's volumes into a named snapshot.
    Save {
        name: String,
        #[arg(default_value = "baseline")]
        label: String,
    },

    /// List saved snapshots.
    List,

    /// Delete a snapshot.
    Rm { label: String },
}

pub fn dispatch(cmd: SnapshotCmd, root: &Path) -> Result<(), String> {
    match cmd {
        SnapshotCmd::Save { name, label } => save(root, &name, &label),
        SnapshotCmd::List => list(root),
        SnapshotCmd::Rm { label } => rm(root, &label),
    }
}

/// One snapshot's metadata, stored next to its volume tarballs.
#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    label: String,
    source_berth: String,
    created_at: u64,
    /// Logical (Compose) volume names, one `<name>.tar` per entry.
    volumes: Vec<String>,
}

/// Capture a berth's Docker volumes into `.berth/snapshots/<label>/`.
pub fn save(root: &Path, name: &str, label: &str) -> Result<(), String> {
    validate_label(label)?;
    let berth = state::get(root, name)?.ok_or_else(|| format!("Berth '{name}' does not exist."))?;
    let project = &berth.compose_project;

    let volumes = docker::project_volumes(project)?;
    if volumes.is_empty() {
        return Err(format!(
            "Berth '{name}' has no volumes to snapshot (run `berth up` or `berth start` first)."
        ));
    }

    // Optional `snapshot.volumes` filter from berth.yml; default is every volume.
    let keep = configured_volumes(root);
    let selected: Vec<(String, String)> = volumes
        .iter()
        .map(|actual| (actual.clone(), logical_name(actual, project)))
        .filter(|(_, logical)| keep.as_ref().is_none_or(|k| k.contains(logical)))
        .collect();
    if selected.is_empty() {
        return Err("No volumes matched the `snapshot.volumes` filter in berth.yml.".to_string());
    }

    let dir = snapshot_dir(root, label);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create snapshot directory: {e}"))?;
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("Failed to resolve snapshot directory: {e}"))?;

    // Stop for a consistent copy (a hot copy of live DB files can be corrupt),
    // then restore the berth's prior run state.
    let running = is_running(project);
    let env_file = env_file(root, name);
    if running {
        docker::stop(root, &berth.compose_file, &env_file, project)?;
    }

    let captured = (|| {
        for (actual, logical) in &selected {
            docker::backup_volume(actual, &dir, &format!("{logical}.tar"))?;
        }
        write_manifest(
            &dir,
            &Manifest {
                label: label.to_string(),
                source_berth: name.to_string(),
                created_at: now(),
                volumes: selected.iter().map(|(_, l)| l.clone()).collect(),
            },
        )
    })();

    if running {
        let _ = docker::up(root, &berth.compose_file, &env_file, project);
    }

    captured?;
    println!(
        "✓ Snapshot '{label}' saved from '{name}' ({} volume(s)).",
        selected.len()
    );
    Ok(())
}

/// Restore a snapshot into a berth's volumes, replacing their contents.
pub fn restore(root: &Path, name: &str, label: &str) -> Result<(), String> {
    validate_label(label)?;
    let berth = state::get(root, name)?.ok_or_else(|| format!("Berth '{name}' does not exist."))?;
    let project = &berth.compose_project;

    let dir = snapshot_dir(root, label);
    let manifest = read_manifest(&dir).map_err(|e| format!("Snapshot '{label}' not found: {e}"))?;
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("Failed to resolve snapshot directory: {e}"))?;

    let volumes = docker::project_volumes(project)?;
    if volumes.is_empty() {
        return Err(format!(
            "Berth '{name}' has no volumes (run `berth up` or `berth start` first)."
        ));
    }
    let by_logical: HashMap<String, String> = volumes
        .iter()
        .map(|actual| (logical_name(actual, project), actual.clone()))
        .collect();

    // Overwriting a live volume is unsafe, so stop first; a reset is meant to be
    // used, so bring the berth back up afterward.
    let env_file = env_file(root, name);
    docker::stop(root, &berth.compose_file, &env_file, project)?;

    let restored = (|| {
        let mut count = 0;
        for logical in &manifest.volumes {
            match by_logical.get(logical) {
                Some(actual) => {
                    docker::restore_volume(actual, &dir, &format!("{logical}.tar"))?;
                    count += 1;
                }
                None => {
                    eprintln!("  ! snapshot volume '{logical}' has no match in '{name}', skipped")
                }
            }
        }
        Ok::<usize, String>(count)
    })();

    let started = docker::up(root, &berth.compose_file, &env_file, project);

    let restored = restored?;
    started?;
    println!("✓ Berth '{name}' reset to snapshot '{label}' ({restored} volume(s)).");
    Ok(())
}

pub fn list(root: &Path) -> Result<(), String> {
    let base = root.join(SNAPSHOTS_DIR);

    let mut snapshots: Vec<Manifest> = match fs::read_dir(&base) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| read_manifest(&e.path()).ok())
            .collect(),
        Err(_) => Vec::new(),
    };

    if snapshots.is_empty() {
        println!("No snapshots.");
        return Ok(());
    }

    snapshots.sort_by_key(|m| std::cmp::Reverse(m.created_at));

    println!("{:<16} {:<14} {:<6} VOLUMES", "LABEL", "SOURCE", "AGE");
    for m in snapshots {
        println!(
            "{:<16} {:<14} {:<6} {}",
            m.label,
            m.source_berth,
            ls::format_age(m.created_at),
            m.volumes.join(","),
        );
    }
    Ok(())
}

pub fn rm(root: &Path, label: &str) -> Result<(), String> {
    validate_label(label)?;
    let dir = snapshot_dir(root, label);
    if !dir.exists() {
        return Err(format!("Snapshot '{label}' does not exist."));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("Failed to remove snapshot: {e}"))?;
    println!("✓ Snapshot '{label}' removed.");
    Ok(())
}

/// Map an actual Docker volume name to its logical (Compose) name by stripping
/// the `<project>_` prefix Compose adds.
fn logical_name(actual: &str, project: &str) -> String {
    actual
        .strip_prefix(&format!("{project}_"))
        .unwrap_or(actual)
        .to_string()
}

fn configured_volumes(root: &Path) -> Option<HashSet<String>> {
    validate::validate(root.to_path_buf())
        .ok()
        .and_then(|config| config.snapshot)
        .and_then(|snapshot| snapshot.volumes)
        .map(|volumes| volumes.into_iter().collect())
}

fn is_running(project: &str) -> bool {
    docker::running_projects()
        .map(|running| running.contains(project))
        .unwrap_or(false)
}

fn snapshot_dir(root: &Path, label: &str) -> PathBuf {
    root.join(SNAPSHOTS_DIR).join(label)
}

fn env_file(root: &Path, name: &str) -> PathBuf {
    root.join(".berth").join(format!("{name}.env"))
}

/// Labels become directory names, so keep them to a safe single path segment.
fn validate_label(label: &str) -> Result<(), String> {
    let ok =
        !label.is_empty() && !label.contains('/') && !label.contains('\\') && !label.contains("..");
    if ok {
        Ok(())
    } else {
        Err(format!(
            "Invalid snapshot label '{label}' — must be a single path segment with no '/', '\\' or '..'."
        ))
    }
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let json = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    fs::write(dir.join("manifest.json"), json)
        .map_err(|e| format!("Failed to write snapshot manifest: {e}"))
}

fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let contents = fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?;
    serde_json::from_str(&contents).map_err(|e| format!("invalid manifest: {e}"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_name_strips_project_prefix() {
        assert_eq!(
            logical_name("berth-alpha_postgres-data", "berth-alpha"),
            "postgres-data"
        );
        // A name without the prefix is returned unchanged.
        assert_eq!(
            logical_name("some-external-vol", "berth-alpha"),
            "some-external-vol"
        );
    }

    #[test]
    fn label_validation() {
        assert!(validate_label("baseline").is_ok());
        assert!(validate_label("v1.2_clean").is_ok());
        assert!(validate_label("").is_err());
        assert!(validate_label("a/b").is_err());
        assert!(validate_label("..").is_err());
        assert!(validate_label("../escape").is_err());
    }

    #[test]
    fn manifest_round_trips() {
        let manifest = Manifest {
            label: "baseline".into(),
            source_berth: "alpha".into(),
            created_at: 123,
            volumes: vec!["postgres-data".into(), "redis".into()],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.label, "baseline");
        assert_eq!(back.volumes, vec!["postgres-data", "redis"]);
    }

    #[test]
    fn snapshot_dir_is_under_berth() {
        let dir = snapshot_dir(Path::new("/repo"), "baseline");
        assert_eq!(dir, Path::new("/repo/.berth/snapshots/baseline"));
    }
}
