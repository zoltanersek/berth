use std::path::{Path, PathBuf};
use std::process::Command;

/// A berth resolved from a working directory inside its worktree.
pub struct Resolved {
    /// The main worktree root, where `.berth/state.json` lives.
    pub root: PathBuf,
    /// The berth name — equal to the worktree's branch (see `up::up`).
    pub name: String,
}

/// Resolve the berth for `cwd` from git. Works whether `cwd` is the main
/// worktree or a linked one; the state directory always lives in the main
/// worktree, which we find via the shared (common) git directory.
pub fn resolve(cwd: &Path) -> Result<Resolved, String> {
    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let name = branch_to_name(&branch)?;

    let common = git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let root = root_from_common_dir(&common)?;

    Ok(Resolved { root, name })
}

fn branch_to_name(branch: &str) -> Result<String, String> {
    let branch = branch.trim();
    if branch.is_empty() || branch == "HEAD" {
        return Err("detached HEAD — no branch to map to a berth".to_string());
    }
    Ok(branch.to_string())
}

/// The main worktree root is the parent of the common git directory
/// (`<root>/.git`), even when called from a linked worktree.
fn root_from_common_dir(common_dir: &str) -> Result<PathBuf, String> {
    Path::new(common_dir.trim())
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "could not resolve the main worktree root".to_string())
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_rejects_detached_head() {
        assert!(branch_to_name("HEAD").is_err());
        assert!(branch_to_name("   ").is_err());
        assert_eq!(branch_to_name("feature/x\n").unwrap(), "feature/x");
    }

    #[test]
    fn root_is_parent_of_common_git_dir() {
        assert_eq!(
            root_from_common_dir("/home/me/repo/.git").unwrap(),
            PathBuf::from("/home/me/repo")
        );
        assert_eq!(
            root_from_common_dir("/home/me/repo/.git\n").unwrap(),
            PathBuf::from("/home/me/repo")
        );
    }
}
