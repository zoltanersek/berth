use std::{path::Path, process::Command};

pub fn check(root: &Path) -> Result<(), String> {
    git()?;
    docker()?;
    docker_compose()?;
    git_repo(root)?;
    git_head(root)?;

    Ok(())
}

pub fn validate_compose(root: &Path, compose: &Path, env_file: &Path) -> Result<(), String> {
    // Pass the env file so Berth-injected port variables resolve; otherwise
    // `${PORT}` interpolations are empty and `config` reports bogus errors.
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose)
        .arg("--env-file")
        .arg(env_file)
        .arg("config")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Invalid docker compose configuration:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

fn git() -> Result<(), String> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .map_err(|_| "Git is not installed or not in PATH.".to_string())?;

    if !output.status.success() {
        return Err("Git is not available.".into());
    }

    Ok(())
}

fn docker() -> Result<(), String> {
    let output = Command::new("docker")
        .arg("info")
        .output()
        .map_err(|_| "Docker is not installed or not in PATH.".to_string())?;

    if !output.status.success() {
        return Err(
            "Docker is installed but the daemon is not running.\nStart Docker Desktop (or the Docker daemon) and try again."
                .into(),
        );
    }

    Ok(())
}

fn docker_compose() -> Result<(), String> {
    let output = Command::new("docker")
        .arg("compose")
        .arg("version")
        .output()
        .map_err(|_| "Docker Compose is not available.".to_string())?;

    if !output.status.success() {
        return Err("Docker Compose is not available.".into());
    }

    Ok(())
}

fn git_repo(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "'{}' is not inside a git repository.",
            root.display()
        ));
    }

    Ok(())
}

fn git_head(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--verify")
        .arg("HEAD")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute git: {e}"))?;

    if !output.status.success() {
        return Err(
            "Repository has no commits.\nCreate an initial commit before using Berth.".into(),
        );
    }

    Ok(())
}
