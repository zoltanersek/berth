use std::{path::Path, process::Command};

pub fn up(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project_name: &str,
) -> Result<(), String> {
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("--env-file")
        .arg(env_file)
        .arg("-p")
        .arg(project_name)
        .arg("up")
        .arg("-d")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(())
}

pub fn down(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project_name: &str,
) -> Result<(), String> {
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("--env-file")
        .arg(env_file)
        .arg("-p")
        .arg(project_name)
        .arg("down")
        .arg("-v")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(())
}
