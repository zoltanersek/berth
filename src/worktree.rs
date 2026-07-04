use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn create(root: &Path, name: &str) -> Result<PathBuf, String> {
    // Canonicalize first so a relative root like "." resolves to an absolute
    // path with a real final component (otherwise `.file_name()` is None).
    let root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project directory: {e}"))?;

    let repo_name = root
        .file_name()
        .ok_or("Invalid repository path")?
        .to_string_lossy();

    let parent = root.parent().ok_or("Repository has no parent directory")?;

    let worktree = parent.join(format!("{}-{}", repo_name, name));

    let output = Command::new("git")
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(name)
        .arg(&worktree)
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(worktree)
}

/// Verify a berth's worktree is safe to remove: no uncommitted changes and the
/// branch is merged into the current branch. Returns an error describing the
/// blocker (and suggesting `--force`) otherwise. Call this *before* tearing down
/// any resources so a refusal leaves the berth fully intact.
pub fn ensure_removable(root: &Path, worktree: &Path, branch: &str) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .arg("status")
        .arg("--porcelain")
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    if !output.stdout.is_empty() {
        return Err(format!(
            "Berth '{branch}' has uncommitted changes. Commit them, or run `berth down {branch} --force` to discard.",
        ));
    }

    // Use --format to get bare branch names. Plain `git branch --merged`
    // decorates lines with markers (`* ` for the current branch, `+ ` for a
    // branch checked out in a linked worktree) — and a berth's branch is always
    // checked out in its worktree, so it always carries the `+ ` marker.
    let output = Command::new("git")
        .arg("branch")
        .arg("--format=%(refname:short)")
        .arg("--merged")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let merged = String::from_utf8_lossy(&output.stdout);

    let is_merged = merged.lines().map(|l| l.trim()).any(|b| b == branch);

    if !is_merged {
        return Err(format!(
            "Branch '{branch}' is not merged into the current branch. Merge it, or run `berth down {branch} --force` to discard.",
        ));
    }

    Ok(())
}

/// Remove the worktree and its branch. When `force` is true the worktree is
/// removed even if dirty (`git worktree remove --force`) and the branch is
/// force-deleted (`git branch -D`) even if unmerged — used to tear down an
/// abandoned berth. When false, callers should have run [`ensure_removable`]
/// first.
pub fn remove(root: &Path, worktree: &Path, branch: &str, force: bool) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.arg("worktree").arg("remove");
    if force {
        cmd.arg("--force");
    }
    cmd.arg(worktree).current_dir(root);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let output = Command::new("git")
        .arg("branch")
        .arg(if force { "-D" } else { "-d" })
        .arg(branch)
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(())
}
