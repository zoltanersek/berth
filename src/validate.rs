use std::{fs, path::PathBuf};

use crate::types::Config;

pub fn validate(root: PathBuf) -> Result<Config, Vec<String>> {
    let mut errors = Vec::new();

    let config_path = root.join("berth.yml");

    if !config_path.exists() {
        return Err(vec!["berth.yml not found".to_string()]);
    }

    let contents = match fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(e) => {
            return Err(vec![format!("Failed to read berth.yml: {e}")]);
        }
    };

    let config: Config = match serde_yaml::from_str(&contents) {
        Ok(config) => config,
        Err(e) => {
            return Err(vec![format!("Invalid YAML: {e}")]);
        }
    };

    if config.version != 1 {
        errors.push(format!(
            "Unsupported config version {} (expected 1)",
            config.version
        ));
    }

    let compose_path = root.join(&config.compose);

    if !compose_path.exists() {
        errors.push(format!(
            "Compose file '{}' does not exist",
            compose_path.display()
        ));
    }

    for (name, port) in &config.ports {
        if port.env.trim().is_empty() {
            errors.push(format!("Port '{}' has an empty env field", name));
        }
    }

    if errors.is_empty() {
        Ok(config)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, contents: &str) {
        fs::write(dir.path().join(name), contents).unwrap();
    }

    #[test]
    fn valid_config() {
        let dir = TempDir::new().unwrap();
        write(&dir, "docker-compose.yml", "services: {}\n");
        write(
            &dir,
            "berth.yml",
            "version: 1\ncompose: docker-compose.yml\nports:\n  web:\n    env: WEB_PORT\n",
        );

        let config = validate(dir.path().to_path_buf()).expect("should be valid");
        assert_eq!(config.version, 1);
        assert_eq!(config.compose, "docker-compose.yml");
        assert_eq!(config.ports["web"].env, "WEB_PORT");
    }

    #[test]
    fn missing_config_file() {
        let dir = TempDir::new().unwrap();
        let errs = validate(dir.path().to_path_buf()).unwrap_err();
        assert!(errs[0].contains("berth.yml not found"));
    }

    #[test]
    fn unsupported_version() {
        let dir = TempDir::new().unwrap();
        write(&dir, "docker-compose.yml", "services: {}\n");
        write(
            &dir,
            "berth.yml",
            "version: 2\ncompose: docker-compose.yml\n",
        );

        let errs = validate(dir.path().to_path_buf()).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("Unsupported config version"))
        );
    }

    #[test]
    fn missing_compose_file() {
        let dir = TempDir::new().unwrap();
        write(&dir, "berth.yml", "version: 1\ncompose: nope.yml\n");

        let errs = validate(dir.path().to_path_buf()).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("does not exist")));
    }

    #[test]
    fn empty_port_env_rejected() {
        let dir = TempDir::new().unwrap();
        write(&dir, "docker-compose.yml", "services: {}\n");
        write(
            &dir,
            "berth.yml",
            "version: 1\ncompose: docker-compose.yml\nports:\n  web:\n    env: \"  \"\n",
        );

        let errs = validate(dir.path().to_path_buf()).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("empty env field")));
    }

    #[test]
    fn unknown_field_rejected() {
        let dir = TempDir::new().unwrap();
        write(&dir, "docker-compose.yml", "services: {}\n");
        write(
            &dir,
            "berth.yml",
            "version: 1\ncompose: docker-compose.yml\nbogus: true\n",
        );

        let errs = validate(dir.path().to_path_buf()).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("Invalid YAML")));
    }
}
