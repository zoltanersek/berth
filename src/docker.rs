use std::{
    collections::HashSet,
    path::Path,
    process::{Child, Command, Stdio},
};

/// Run `docker compose` for a berth's project with the given trailing args.
/// The `-f`, `--env-file` and `-p` flags are constant across every berth
/// operation, so they live here and callers only supply the verb (`up -d`,
/// `down -v`, `stop`, …).
fn compose(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project: &str,
    args: &[&str],
) -> Result<(), String> {
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("--env-file")
        .arg(env_file)
        .arg("-p")
        .arg(project)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(())
}

pub fn up(root: &Path, compose_file: &Path, env_file: &Path, project: &str) -> Result<(), String> {
    compose(root, compose_file, env_file, project, &["up", "-d"])
}

pub fn down(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project: &str,
) -> Result<(), String> {
    compose(root, compose_file, env_file, project, &["down", "-v"])
}

/// Stop a berth's containers without removing them, so `restart`/`up` can bring
/// the same containers back. Deliberately not `down -v`, which destroys them.
pub fn stop(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project: &str,
) -> Result<(), String> {
    compose(root, compose_file, env_file, project, &["stop"])
}

pub fn restart(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project: &str,
) -> Result<(), String> {
    compose(root, compose_file, env_file, project, &["restart"])
}

/// Return the raw stdout of `docker compose ps --format json` for a project.
/// The output format varies by Compose version (a JSON array on some, one JSON
/// object per line on others), so parsing is left to the caller.
pub fn compose_ps(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project: &str,
) -> Result<String, String> {
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("--env-file")
        .arg(env_file)
        .arg("-p")
        .arg(project)
        .arg("ps")
        .arg("--format")
        .arg("json")
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Spawn a `docker compose logs -f` child with its stdout piped, for streaming
/// to a dashboard client. The caller owns the child's lifecycle and must kill
/// and reap it when the stream ends (see the dashboard's SSE handler).
pub fn logs_child(
    root: &Path,
    compose_file: &Path,
    env_file: &Path,
    project: &str,
) -> Result<Child, String> {
    Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("--env-file")
        .arg(env_file)
        .arg("-p")
        .arg(project)
        .arg("logs")
        .arg("-f")
        .arg("--tail")
        .arg("200")
        .arg("--no-color")
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to execute docker: {e}"))
}

/// The set of Compose project names with at least one running container, keyed
/// by the `com.docker.compose.project` label. A berth is "running" when its
/// `compose_project` is in this set. One `docker ps` covers every berth.
pub fn running_projects() -> Result<HashSet<String>, String> {
    let output = Command::new("docker")
        .args([
            "ps",
            "--format",
            "{{.Label \"com.docker.compose.project\"}}",
        ])
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

/// Actual Docker volume names belonging to a Compose project (via its
/// `com.docker.compose.project` label).
pub fn project_volumes(project: &str) -> Result<Vec<String>, String> {
    let output = Command::new("docker")
        .args([
            "volume",
            "ls",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--format",
            "{{.Name}}",
        ])
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

/// Tar the contents of a Docker volume into `<host_dir>/<tar_name>` using a
/// throwaway `busybox` container. `tar` preserves uid/gid, so file ownership
/// (e.g. Postgres' `999`) survives the round trip.
pub fn backup_volume(volume: &str, host_dir: &Path, tar_name: &str) -> Result<(), String> {
    volume_copy(&[
        "-v",
        &format!("{volume}:/from:ro"),
        "-v",
        &format!("{}:/backup", host_dir.display()),
        "busybox",
        "tar",
        "cf",
        &format!("/backup/{tar_name}"),
        "-C",
        "/from",
        ".",
    ])
}

/// Replace a Docker volume's contents with those of `<host_dir>/<tar_name>`.
pub fn restore_volume(volume: &str, host_dir: &Path, tar_name: &str) -> Result<(), String> {
    volume_copy(&[
        "-v",
        &format!("{volume}:/to"),
        "-v",
        &format!("{}:/backup", host_dir.display()),
        "busybox",
        "sh",
        "-c",
        &format!("rm -rf /to/* /to/.[!.]* /to/..?* 2>/dev/null; tar xf /backup/{tar_name} -C /to"),
    ])
}

fn volume_copy(args: &[&str]) -> Result<(), String> {
    let output = Command::new("docker")
        .args(["run", "--rm"])
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute docker: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(())
}
