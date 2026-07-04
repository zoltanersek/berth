use std::path::Path;

use crate::{docker, env, state, worktree};

pub fn down(root: &Path, name: &str, force: bool) -> Result<(), String> {
    let berth =
        state::get(root, name)?.ok_or_else(|| format!("Berth '{}' does not exist.", name))?;

    // Refuse up front (before tearing anything down) unless forced, so a
    // rejected teardown leaves the berth fully intact.
    if !force {
        worktree::ensure_removable(root, &berth.worktree, &berth.branch)?;
    }

    // Best-effort cleanup. We try everything and report all failures.
    let mut errors = Vec::new();

    let env_file = root.join(".berth").join(format!("{name}.env"));

    if let Err(e) = docker::down(root, &berth.compose_file, &env_file, &berth.compose_project) {
        errors.push(e);
    }

    if let Err(e) = env::remove(root, name) {
        errors.push(e);
    }

    // Only drop the state entry once the worktree is actually gone, otherwise a
    // failed removal would leave an untracked worktree + branch on disk.
    match worktree::remove(root, &berth.worktree, &berth.branch, force) {
        Ok(()) => {
            if let Err(e) = state::remove(root, name) {
                errors.push(e);
            }
        }
        Err(e) => errors.push(e),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
