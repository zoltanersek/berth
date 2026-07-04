use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{docker, doctor, env, ports, state, types::Berth, validate, worktree};

struct Rollback {
    worktree: Option<PathBuf>,
    env_created: bool,
    docker_started: bool,
}

impl Rollback {
    fn new() -> Self {
        Self {
            worktree: None,
            env_created: false,
            docker_started: false,
        }
    }

    fn cleanup(&self, root: &Path, name: &str, compose: &Path, project: &str) {
        let env_file = root.join(".berth").join(format!("{name}.env"));

        if self.docker_started {
            let _ = docker::down(root, compose, &env_file, project);
        }

        if self.env_created {
            let _ = env::remove(root, name);
        }

        if let Some(worktree) = &self.worktree {
            // Force: this is a fresh, aborted berth being rolled back.
            let _ = worktree::remove(root, worktree, name, true);
        }

        let _ = state::remove(root, name);
    }
}

pub fn up(root: &Path, name: &str) -> Result<(), String> {
    doctor::check(root)?;

    let config = validate::validate(root.to_path_buf()).map_err(|errs| errs.join("\n"))?;

    if state::get(root, name)?.is_some() {
        return Err(format!("Berth '{name}' already exists."));
    }

    let compose = root.join(&config.compose);
    // Store an absolute path so teardown works regardless of the caller's cwd.
    let compose_file = compose.canonicalize().unwrap_or_else(|_| compose.clone());
    let project = format!("berth-{name}");
    let env_file = root.join(".berth").join(format!("{name}.env"));

    let mut rollback = Rollback::new();

    let result: Result<Berth, String> = (|| {
        let ports = ports::allocate(&config)?;

        // Generate the env first so compose validation can resolve the injected
        // port variables. This only writes .berth/<name>.env, which rolls back
        // cleanly if validation fails — no worktree/branch created yet.
        env::generate(root, name, &config, &ports)?;
        rollback.env_created = true;

        doctor::validate_compose(root, &compose, &env_file)?;

        let worktree = worktree::create(root, name)?;
        rollback.worktree = Some(worktree.clone());

        docker::up(root, &compose, &env_file, &project)?;
        rollback.docker_started = true;

        Ok(Berth {
            name: name.to_string(),
            branch: name.to_string(),
            worktree,
            compose_project: project.clone(),
            compose_file: compose_file.clone(),
            ports,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        })
    })();

    match result {
        Ok(berth) => {
            if let Err(err) = state::upsert(root, berth) {
                rollback.cleanup(root, name, &compose, &project);
                return Err(err);
            }

            let berth = state::get(root, name)?.unwrap();

            println!("Worktree:");
            println!("  {}", berth.worktree.display());

            if !berth.ports.is_empty() {
                let mut ports: Vec<_> = berth.ports.iter().collect();
                ports.sort_by_key(|(k, _)| *k);

                println!();
                println!("Ports:");
                for (env, port) in &ports {
                    println!("  {:<20} {}", env, port);
                }

                println!();
                println!("URLs:");
                for (_, port) in &ports {
                    println!("  http://localhost:{port}");
                }
            }

            println!();

            Ok(())
        }

        Err(err) => {
            rollback.cleanup(root, name, &compose, &project);
            Err(err)
        }
    }
}
