use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// A supported agent harness and where its hook config lives.
#[derive(Debug, Clone, Copy)]
pub enum Harness {
    Claude,
    Codex,
}

impl Harness {
    /// The (settings event name, `berth hooks run` argument) pairs to install.
    /// Codex has no `SessionEnd` event, so only its start side is wired up.
    fn events(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Harness::Claude => &[
                ("SessionStart", "session-start"),
                ("SessionEnd", "session-end"),
            ],
            Harness::Codex => &[("SessionStart", "session-start")],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Harness::Claude => "Claude Code",
            Harness::Codex => "Codex",
        }
    }

    fn settings_path(self, root: &Path, global: bool) -> Result<PathBuf, String> {
        let (dir, file) = match self {
            Harness::Claude => (".claude", "settings.json"),
            Harness::Codex => (".codex", "hooks.json"),
        };
        let base = if global {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            PathBuf::from(home)
        } else {
            root.to_path_buf()
        };
        Ok(base.join(dir).join(file))
    }
}

pub fn install(root: &Path, harnesses: &[Harness], global: bool) -> Result<(), String> {
    let command = hook_command(global)?;

    for &harness in harnesses {
        let path = harness.settings_path(root, global)?;
        let mut settings = load(&path)?;

        if merge(&mut settings, harness.events(), &command)? {
            write(&path, &settings)?;
            println!(
                "✓ {}: hooks installed → {}",
                harness.label(),
                path.display()
            );
        } else {
            println!(
                "• {}: already installed → {}",
                harness.label(),
                path.display()
            );
        }

        if matches!(harness, Harness::Codex) {
            println!(
                "  note: Codex has no session-end hook — use `berth agent <name> -- codex` for teardown."
            );
        }
    }

    Ok(())
}

pub fn uninstall(root: &Path, harnesses: &[Harness], global: bool) -> Result<(), String> {
    for &harness in harnesses {
        let path = harness.settings_path(root, global)?;
        let mut settings = load(&path)?;

        if strip(&mut settings)? {
            write(&path, &settings)?;
            println!("✓ {}: hooks removed → {}", harness.label(), path.display());
        } else {
            println!(
                "• {}: nothing to remove → {}",
                harness.label(),
                path.display()
            );
        }
    }

    Ok(())
}

/// The command the harness should invoke. For machine-global installs we bake
/// in the absolute path to this binary; for project (committed) settings we use
/// bare `berth` so the file stays portable across machines.
fn hook_command(global: bool) -> Result<String, String> {
    if global {
        let exe = std::env::current_exe().map_err(|e| format!("Cannot resolve berth path: {e}"))?;
        Ok(exe.display().to_string())
    } else {
        Ok("berth".to_string())
    }
}

/// Insert berth's hook entries idempotently, preserving every other key and any
/// existing hooks. Returns whether anything changed.
fn merge(settings: &mut Value, events: &[(&str, &str)], command: &str) -> Result<bool, String> {
    if settings.is_null() {
        *settings = json!({});
    }
    let obj = settings
        .as_object_mut()
        .ok_or("settings root is not a JSON object")?;

    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()));
    let hooks = hooks
        .as_object_mut()
        .ok_or("`hooks` is not a JSON object")?;

    let mut changed = false;
    for (event, arg) in events {
        let full = format!("{command} hooks run {arg}");
        let array = hooks
            .entry(*event)
            .or_insert_with(|| Value::Array(Vec::new()));
        let array = array
            .as_array_mut()
            .ok_or_else(|| format!("`hooks.{event}` is not an array"))?;

        if !array.iter().any(|group| group_has_command(group, &full)) {
            array.push(json!({ "hooks": [ { "type": "command", "command": full } ] }));
            changed = true;
        }
    }

    Ok(changed)
}

/// Remove berth's hook entries (identified by a `hooks run session-*` command),
/// leaving foreign entries untouched. Returns whether anything changed.
fn strip(settings: &mut Value) -> Result<bool, String> {
    let Some(obj) = settings.as_object_mut() else {
        return Ok(false);
    };

    let mut changed = false;
    let mut hooks_empty = false;

    if let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_object_mut) {
        for event in hooks.keys().cloned().collect::<Vec<_>>() {
            if let Some(array) = hooks.get_mut(&event).and_then(Value::as_array_mut) {
                let before = array.len();
                array.retain(|group| !group_is_berth(group));
                if array.len() != before {
                    changed = true;
                }
                if array.is_empty() {
                    hooks.remove(&event);
                }
            }
        }
        hooks_empty = hooks.is_empty();
    }

    if changed && hooks_empty {
        obj.remove("hooks");
    }

    Ok(changed)
}

fn group_has_command(group: &Value, command: &str) -> bool {
    inner_commands(group).any(|c| c == command)
}

fn group_is_berth(group: &Value) -> bool {
    inner_commands(group)
        .any(|c| c.contains("hooks run session-start") || c.contains("hooks run session-end"))
}

fn inner_commands(group: &Value) -> impl Iterator<Item = &str> {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(Value::as_str))
}

fn load(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => Ok(json!({})),
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

fn write(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let mut json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    json.push('\n');
    std::fs::write(path, json).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_idempotent_and_additive() {
        let mut settings = json!({});
        assert!(merge(&mut settings, Harness::Claude.events(), "berth").unwrap());
        // Re-running makes no further change.
        assert!(!merge(&mut settings, Harness::Claude.events(), "berth").unwrap());

        let start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 1);
        assert_eq!(
            start[0]["hooks"][0]["command"],
            "berth hooks run session-start"
        );
        assert_eq!(
            settings["hooks"]["SessionEnd"][0]["hooks"][0]["command"],
            "berth hooks run session-end"
        );
    }

    #[test]
    fn merge_preserves_other_keys_and_foreign_hooks() {
        let mut settings = json!({
            "model": "sonnet",
            "hooks": {
                "SessionStart": [ { "hooks": [ { "type": "command", "command": "other-tool" } ] } ]
            }
        });
        merge(&mut settings, Harness::Claude.events(), "berth").unwrap();

        assert_eq!(settings["model"], "sonnet");
        let start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 2, "foreign hook kept, berth hook added");
    }

    #[test]
    fn codex_only_wires_session_start() {
        let events = Harness::Codex.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "SessionStart");
    }

    #[test]
    fn strip_removes_only_berth_entries() {
        let mut settings = json!({ "model": "sonnet" });
        merge(&mut settings, Harness::Claude.events(), "berth").unwrap();
        settings["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "hooks": [ { "type": "command", "command": "keep-me" } ] }));

        assert!(strip(&mut settings).unwrap());

        let start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 1);
        assert_eq!(start[0]["hooks"][0]["command"], "keep-me");
        // SessionEnd held only berth's entry, so it's gone entirely.
        assert!(settings["hooks"].get("SessionEnd").is_none());
        assert_eq!(settings["model"], "sonnet");
    }

    #[test]
    fn strip_without_berth_entries_is_noop() {
        let mut settings = json!({ "hooks": { "SessionStart": [] } });
        assert!(!strip(&mut settings).unwrap());
    }

    #[test]
    fn malformed_hooks_value_errors() {
        let mut settings = json!({ "hooks": "not-an-object" });
        assert!(merge(&mut settings, Harness::Claude.events(), "berth").is_err());
    }
}
