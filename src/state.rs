use crate::types::{Berth, BerthState};
use fs2::FileExt;
use std::{fs, fs::File, path::Path};

const BERTH_DIR: &str = ".berth";
const STATE_FILE: &str = ".berth/state.json";
const LOCK_FILE: &str = ".berth/state.lock";

/// Run `f` while holding an exclusive advisory lock on the state directory.
///
/// This serializes the read-modify-write of `state.json` across concurrent
/// `berth up` / `berth down` invocations — the core parallel-agent scenario.
/// The lock is an OS advisory lock tied to the file handle, so it is released
/// automatically when the process exits, even on a crash (no stale lockfiles).
fn with_lock<T>(root: &Path, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let dir = root.join(BERTH_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create .berth directory: {e}"))?;

    let lock = File::create(root.join(LOCK_FILE))
        .map_err(|e| format!("Failed to open state lock: {e}"))?;

    lock.lock_exclusive()
        .map_err(|e| format!("Failed to acquire state lock: {e}"))?;

    // The lock is held until `lock` is dropped at the end of this scope.
    f()
}

pub fn load(root: &Path) -> Result<BerthState, String> {
    let path = root.join(STATE_FILE);

    if !path.exists() {
        return Ok(BerthState::default());
    }

    let contents = fs::read_to_string(&path).map_err(|e| format!("Failed to read state: {e}"))?;

    serde_json::from_str(&contents).map_err(|e| format!("Invalid state.json: {e}"))
}

pub fn save(root: &Path, state: &BerthState) -> Result<(), String> {
    let dir = root.join(BERTH_DIR);

    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create .berth directory: {e}"))?;

    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;

    // Write to a temp file then rename, so a crash mid-write cannot leave a
    // truncated/corrupt state.json — readers always see a complete file.
    let tmp = dir.join(".state.json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Failed to write state: {e}"))?;
    fs::rename(&tmp, dir.join("state.json")).map_err(|e| format!("Failed to save state: {e}"))
}

pub fn upsert(root: &Path, berth: Berth) -> Result<(), String> {
    with_lock(root, || {
        let mut state = load(root)?;

        state.berths.retain(|b| b.name != berth.name);
        state.berths.push(berth);

        save(root, &state)
    })
}

pub fn remove(root: &Path, name: &str) -> Result<(), String> {
    with_lock(root, || {
        let mut state = load(root)?;

        state.berths.retain(|b| b.name != name);

        save(root, &state)
    })
}

pub fn get(root: &Path, name: &str) -> Result<Option<Berth>, String> {
    let state = load(root)?;

    Ok(state.berths.into_iter().find(|berth| berth.name == name))
}

pub fn list(root: &Path) -> Result<Vec<Berth>, String> {
    Ok(load(root)?.berths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn berth(name: &str) -> Berth {
        Berth {
            name: name.to_string(),
            branch: name.to_string(),
            worktree: PathBuf::from(format!("/tmp/{name}")),
            compose_project: format!("berth-{name}"),
            compose_file: PathBuf::from("docker-compose.yml"),
            ports: HashMap::from([("WEB_PORT".to_string(), 5000u16)]),
            created_at: 42,
        }
    }

    #[test]
    fn upsert_get_list_remove_roundtrip() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        assert!(list(root).unwrap().is_empty());

        upsert(root, berth("alpha")).unwrap();
        upsert(root, berth("beta")).unwrap();

        assert_eq!(list(root).unwrap().len(), 2);
        assert!(get(root, "alpha").unwrap().is_some());
        assert!(get(root, "missing").unwrap().is_none());

        remove(root, "alpha").unwrap();
        assert!(get(root, "alpha").unwrap().is_none());
        assert_eq!(list(root).unwrap().len(), 1);
    }

    #[test]
    fn upsert_replaces_existing_by_name() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        upsert(root, berth("alpha")).unwrap();
        let mut updated = berth("alpha");
        updated.branch = "renamed".to_string();
        upsert(root, updated).unwrap();

        assert_eq!(list(root).unwrap().len(), 1);
        assert_eq!(get(root, "alpha").unwrap().unwrap().branch, "renamed");
    }

    #[test]
    fn concurrent_upserts_do_not_lose_entries() {
        // Without the exclusive lock, racing read-modify-write cycles would
        // clobber each other and drop berths. This is the parallel-agent case.
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();

        let handles: Vec<_> = (0..16)
            .map(|i| {
                let root = root.clone();
                std::thread::spawn(move || {
                    upsert(&root, berth(&format!("agent-{i}"))).unwrap();
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(list(&root).unwrap().len(), 16);
    }
}
