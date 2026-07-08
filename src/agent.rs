use std::path::Path;
use std::process::Command;

use crate::{docker, down, state, up};

/// Create a berth, launch an agent command inside its worktree, and tear the
/// berth down (safely) when the command exits.
///
/// This is the harness-agnostic path: a hook cannot relocate a running agent
/// into a new worktree, but launching the command ourselves lets us set its
/// working directory to the fresh worktree. On exit we run a non-force
/// teardown, so a merged, clean branch is cleaned up while in-progress work is
/// kept (with the reason printed).
pub fn agent(root: &Path, name: &str, command: &[String]) -> Result<(), String> {
    let (program, rest) = command
        .split_first()
        .ok_or("No command given. Usage: berth agent <name> -- <cmd> [args...]")?;

    up::up(root, name)?;

    let worktree = state::get(root, name)?
        .ok_or_else(|| format!("Berth '{name}' vanished right after creation"))?
        .worktree;

    println!("Launching '{program}' in {}", worktree.display());
    println!();

    let status = Command::new(program)
        .args(rest)
        .current_dir(&worktree)
        .status()
        .map_err(|e| format!("Failed to launch '{program}': {e}"));

    // Always attempt teardown, even if the launch failed, so we don't strand a
    // berth. A non-force `down` refuses (and keeps) unmerged or dirty work.
    let teardown = down::down(root, name, false);

    let status = status?;

    println!();
    match teardown {
        Ok(()) => println!("Agent exited ({status}). Berth '{name}' torn down."),
        Err(e) if e.contains("does not exist") => {
            println!("Agent exited ({status}). Berth '{name}' already torn down.");
        }
        Err(e) => println!("Agent exited ({status}). Berth '{name}' kept: {e}"),
    }

    Ok(())
}

/// Bring an existing berth's environment up (idempotent).
pub fn start(root: &Path, name: &str) -> Result<(), String> {
    let berth = state::get(root, name)?.ok_or_else(|| format!("Berth '{name}' does not exist."))?;
    let env_file = env_file(root, name);
    docker::up(root, &berth.compose_file, &env_file, &berth.compose_project)?;
    println!("✓ Berth '{name}' environment started.");
    Ok(())
}

/// Stop an existing berth's containers, keeping its worktree and volumes.
pub fn stop(root: &Path, name: &str) -> Result<(), String> {
    let berth = state::get(root, name)?.ok_or_else(|| format!("Berth '{name}' does not exist."))?;
    let env_file = env_file(root, name);
    docker::stop(root, &berth.compose_file, &env_file, &berth.compose_project)?;
    println!("✓ Berth '{name}' environment stopped.");
    Ok(())
}

fn env_file(root: &Path, name: &str) -> std::path::PathBuf {
    root.join(".berth").join(format!("{name}.env"))
}
