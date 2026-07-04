use crate::types::Config;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

const BERTH_DIR: &str = ".berth";
const GITIGNORE_ENTRY: &str = ".berth/";

pub fn generate(
    root: &Path,
    berth_name: &str,
    _config: &Config,
    ports: &HashMap<String, u16>,
) -> Result<PathBuf, String> {
    let dir = root.join(BERTH_DIR);

    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create .berth directory: {e}"))?;

    ensure_gitignore(root)?;

    let path = dir.join(format!("{berth_name}.env"));

    let mut contents = String::new();

    contents.push_str(&format!("BERTH_NAME={berth_name}\n"));
    contents.push_str(&format!("COMPOSE_PROJECT_NAME=berth-{berth_name}\n"));

    let mut vars: Vec<_> = ports.iter().collect();
    vars.sort_by_key(|(k, _)| *k);

    for (env, port) in vars {
        contents.push_str(&format!("{env}={port}\n"));
    }

    fs::write(&path, contents).map_err(|e| format!("Failed to write env file: {e}"))?;

    Ok(path)
}

pub fn remove(root: &Path, berth_name: &str) -> Result<(), String> {
    let path = root.join(BERTH_DIR).join(format!("{berth_name}.env"));

    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("Failed to remove env file: {e}"))?;
    }

    Ok(())
}

fn ensure_gitignore(root: &Path) -> Result<(), String> {
    let gitignore = root.join(".gitignore");

    if !gitignore.exists() {
        fs::write(&gitignore, format!("{GITIGNORE_ENTRY}\n"))
            .map_err(|e| format!("Failed to create .gitignore: {e}"))?;

        return Ok(());
    }

    let mut contents =
        fs::read_to_string(&gitignore).map_err(|e| format!("Failed to read .gitignore: {e}"))?;

    if contents.lines().any(|line| line.trim() == GITIGNORE_ENTRY) {
        return Ok(());
    }

    if !contents.ends_with('\n') {
        contents.push('\n');
    }

    contents.push_str(GITIGNORE_ENTRY);
    contents.push('\n');

    fs::write(&gitignore, contents).map_err(|e| format!("Failed to update .gitignore: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config() -> Config {
        Config {
            version: 1,
            compose: "docker-compose.yml".to_string(),
            ports: HashMap::new(),
        }
    }

    fn ports() -> HashMap<String, u16> {
        HashMap::from([
            ("WEB_PORT".to_string(), 5001u16),
            ("API_PORT".to_string(), 5002u16),
        ])
    }

    #[test]
    fn generate_writes_expected_contents() {
        let dir = TempDir::new().unwrap();
        let path = generate(dir.path(), "alpha", &config(), &ports()).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("BERTH_NAME=alpha"));
        assert!(contents.contains("COMPOSE_PROJECT_NAME=berth-alpha"));
        assert!(contents.contains("API_PORT=5002"));
        assert!(contents.contains("WEB_PORT=5001"));
        // Ports are emitted in sorted key order.
        assert!(
            contents.find("API_PORT").unwrap() < contents.find("WEB_PORT").unwrap(),
            "env vars should be sorted by key"
        );
    }

    #[test]
    fn generate_creates_and_appends_gitignore() {
        let dir = TempDir::new().unwrap();

        // No .gitignore yet -> created with the entry.
        generate(dir.path(), "a", &config(), &ports()).unwrap();
        let gi = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(gi.lines().any(|l| l.trim() == GITIGNORE_ENTRY));

        // Second berth must not duplicate the entry.
        generate(dir.path(), "b", &config(), &ports()).unwrap();
        let gi = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(
            gi.lines().filter(|l| l.trim() == GITIGNORE_ENTRY).count(),
            1
        );
    }

    #[test]
    fn gitignore_preexisting_content_preserved() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules\n").unwrap();

        generate(dir.path(), "a", &config(), &ports()).unwrap();

        let gi = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(gi.contains("node_modules"));
        assert!(gi.lines().any(|l| l.trim() == GITIGNORE_ENTRY));
    }

    #[test]
    fn remove_deletes_env_file() {
        let dir = TempDir::new().unwrap();
        let path = generate(dir.path(), "alpha", &config(), &ports()).unwrap();
        assert!(path.exists());

        remove(dir.path(), "alpha").unwrap();
        assert!(!path.exists());

        // Removing a non-existent berth is a no-op, not an error.
        remove(dir.path(), "ghost").unwrap();
    }
}
