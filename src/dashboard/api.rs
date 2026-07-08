use std::path::Path;

use serde_json::{Value, json};

use super::AppState;
use crate::types::Berth;
use crate::{docker, down, ls, state};

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Index,
    Snapshot,
    Services(String),
    Logs(String),
    Action(String, ActionKind),
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Start,
    Stop,
    Restart,
    Down,
}

/// Map a request method + URL to a route. Pure so it can be exhaustively tested
/// without a running server. The berth `<name>` is taken verbatim here and
/// validated against actual state by the handlers before any Docker call.
pub fn parse_route(method: &str, url: &str) -> Route {
    let path = url.split('?').next().unwrap_or(url).trim_end_matches('/');

    match (method, path) {
        ("GET", "") => Route::Index,
        ("GET", "/api/berths") => Route::Snapshot,
        _ => match path.strip_prefix("/api/berths/").and_then(|rest| {
            // Split off the trailing verb; the remainder is the berth name
            // (which may itself contain `/`, e.g. a `feature/x` branch name).
            rest.rsplit_once('/').filter(|(name, _)| !name.is_empty())
        }) {
            Some((name, "logs")) if method == "GET" => Route::Logs(name.to_string()),
            Some((name, "services")) if method == "GET" => Route::Services(name.to_string()),
            Some((name, verb)) if method == "POST" => match verb {
                "start" => Route::Action(name.to_string(), ActionKind::Start),
                "stop" => Route::Action(name.to_string(), ActionKind::Stop),
                "restart" => Route::Action(name.to_string(), ActionKind::Restart),
                "down" => Route::Action(name.to_string(), ActionKind::Down),
                _ => Route::NotFound,
            },
            _ => Route::NotFound,
        },
    }
}

/// JSON snapshot of every berth: metadata, ports (with URLs), injected env, and
/// a coarse running/stopped status from a single `docker ps`.
pub fn snapshot(state: &AppState) -> (u16, String) {
    let berths = match state::list(&state.root) {
        Ok(berths) => berths,
        Err(e) => return (500, err_json(&e)),
    };

    let running = docker::running_projects().unwrap_or_default();

    let views: Vec<Value> = berths
        .iter()
        .map(|b| {
            let env = read_env(&state.root, &b.name);
            build_berth_view(b, running.contains(&b.compose_project), &env)
        })
        .collect();

    match serde_json::to_string(&json!({ "berths": views })) {
        Ok(json) => (200, json),
        Err(e) => (500, err_json(&e.to_string())),
    }
}

/// Per-service container states for one berth (fetched lazily when a card is
/// expanded, so the poll loop stays a single `docker ps`).
pub fn services(state: &AppState, name: &str) -> (u16, String) {
    let berth = match state::get(&state.root, name) {
        Ok(Some(b)) => b,
        Ok(None) => return (404, err_json("no such berth")),
        Err(e) => return (500, err_json(&e)),
    };

    let env_file = env_file(&state.root, name);

    match docker::compose_ps(
        &state.root,
        &berth.compose_file,
        &env_file,
        &berth.compose_project,
    ) {
        Ok(raw) => {
            let services = service_states(&parse_compose_ps(&raw));
            let body = json!({
                "ok": true,
                "status": derive_status(&services),
                "services": services,
            });
            (200, body.to_string())
        }
        Err(e) => (200, err_json(&e)),
    }
}

/// Run a lifecycle action against a berth. `start`/`stop`/`restart` operate on
/// the Compose project; `down` reuses the full CLI teardown (services, volumes,
/// worktree and branch) and forwards its refusal message when unforced.
pub fn action(state: &AppState, name: &str, kind: ActionKind, body: &str) -> (u16, String) {
    let berth = match state::get(&state.root, name) {
        Ok(Some(b)) => b,
        Ok(None) => return (404, err_json("no such berth")),
        Err(e) => return (500, err_json(&e)),
    };

    let root = &state.root;
    let env_file = env_file(root, name);
    let (file, project) = (&berth.compose_file, &berth.compose_project);

    let result = match kind {
        ActionKind::Start => docker::up(root, file, &env_file, project),
        ActionKind::Stop => docker::stop(root, file, &env_file, project),
        ActionKind::Restart => docker::restart(root, file, &env_file, project),
        ActionKind::Down => down::down(root, name, parse_force(body)),
    };

    match result {
        Ok(()) => (200, json!({ "ok": true }).to_string()),
        Err(e) => (200, err_json(&e)),
    }
}

fn env_file(root: &Path, name: &str) -> std::path::PathBuf {
    root.join(".berth").join(format!("{name}.env"))
}

fn read_env(root: &Path, name: &str) -> Vec<(String, String)> {
    std::fs::read_to_string(env_file(root, name))
        .map(|contents| parse_env_file(&contents))
        .unwrap_or_default()
}

/// Parse a berth env file (the `KEY=VALUE` format written by `env::generate`).
fn parse_env_file(contents: &str) -> Vec<(String, String)> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn build_berth_view(berth: &Berth, running: bool, env: &[(String, String)]) -> Value {
    let mut ports: Vec<(&String, &u16)> = berth.ports.iter().collect();
    ports.sort_by(|a, b| a.0.cmp(b.0));

    let ports: Vec<Value> = ports
        .into_iter()
        .map(|(name, port)| {
            json!({ "env": name, "port": port, "url": format!("http://localhost:{port}") })
        })
        .collect();

    let env: Vec<Value> = env
        .iter()
        .map(|(k, v)| json!({ "key": k, "value": v }))
        .collect();

    json!({
        "name": berth.name,
        "branch": berth.branch,
        "worktree": berth.worktree.display().to_string(),
        "compose_project": berth.compose_project,
        "status": if running { "running" } else { "stopped" },
        "age": ls::format_age(berth.created_at),
        "ports": ports,
        "env": env,
    })
}

/// Parse `docker compose ps --format json`, tolerating both shapes emitted
/// across Compose versions: a single JSON array, or one JSON object per line.
fn parse_compose_ps(stdout: &str) -> Vec<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(trimmed) {
        return items;
    }

    trimmed
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .collect()
}

fn service_states(objects: &[Value]) -> Vec<Value> {
    objects
        .iter()
        .map(|o| {
            let field = |key: &str| o.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            json!({
                "service": field("Service"),
                "state": field("State"),
                "health": field("Health"),
            })
        })
        .collect()
}

fn derive_status(services: &[Value]) -> &'static str {
    if services.is_empty() {
        return "stopped";
    }

    let running = services
        .iter()
        .filter(|s| s.get("state").and_then(Value::as_str) == Some("running"))
        .count();

    if running == 0 {
        "stopped"
    } else if running == services.len() {
        "running"
    } else {
        "partial"
    }
}

fn parse_force(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("force").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn err_json(message: &str) -> String {
    json!({ "ok": false, "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn routes_static_paths() {
        assert_eq!(parse_route("GET", "/"), Route::Index);
        assert_eq!(parse_route("GET", "/?token=x"), Route::Index);
        assert_eq!(parse_route("GET", "/api/berths"), Route::Snapshot);
        assert_eq!(parse_route("GET", "/api/berths?token=x"), Route::Snapshot);
    }

    #[test]
    fn routes_per_berth_endpoints() {
        assert_eq!(
            parse_route("GET", "/api/berths/alpha/logs"),
            Route::Logs("alpha".into())
        );
        assert_eq!(
            parse_route("GET", "/api/berths/alpha/services?token=x"),
            Route::Services("alpha".into())
        );
        assert_eq!(
            parse_route("POST", "/api/berths/alpha/restart"),
            Route::Action("alpha".into(), ActionKind::Restart)
        );
        assert_eq!(
            parse_route("POST", "/api/berths/alpha/down"),
            Route::Action("alpha".into(), ActionKind::Down)
        );
        // A name may contain slashes; only the trailing verb is split off.
        assert_eq!(
            parse_route("POST", "/api/berths/feature/x/stop"),
            Route::Action("feature/x".into(), ActionKind::Stop)
        );
    }

    #[test]
    fn rejects_unknown_and_mismatched_routes() {
        assert_eq!(parse_route("GET", "/nope"), Route::NotFound);
        assert_eq!(
            parse_route("POST", "/api/berths/alpha/logs"),
            Route::NotFound
        );
        assert_eq!(
            parse_route("GET", "/api/berths/alpha/start"),
            Route::NotFound
        );
        assert_eq!(parse_route("POST", "/api/berths//down"), Route::NotFound);
    }

    #[test]
    fn parses_env_file() {
        let env = parse_env_file("# comment\nBERTH_NAME=alpha\n\nWEB_PORT = 5001 \n");
        assert_eq!(
            env,
            vec![
                ("BERTH_NAME".to_string(), "alpha".to_string()),
                ("WEB_PORT".to_string(), "5001".to_string()),
            ]
        );
    }

    #[test]
    fn parse_force_defaults_false() {
        assert!(!parse_force(""));
        assert!(!parse_force("{}"));
        assert!(!parse_force("{\"force\":false}"));
        assert!(parse_force("{\"force\":true}"));
    }

    #[test]
    fn parses_compose_ps_array_and_ndjson() {
        let array = r#"[{"Service":"web","State":"running"},{"Service":"db","State":"exited"}]"#;
        let ndjson = "{\"Service\":\"web\",\"State\":\"running\"}\n{\"Service\":\"db\",\"State\":\"exited\"}";
        for raw in [array, ndjson] {
            let parsed = service_states(&parse_compose_ps(raw));
            assert_eq!(parsed.len(), 2);
            assert_eq!(parsed[0]["service"], "web");
            assert_eq!(parsed[0]["state"], "running");
        }
        assert!(parse_compose_ps("   ").is_empty());
    }

    #[test]
    fn derives_overall_status() {
        let svc = |state: &str| json!({ "state": state });
        assert_eq!(derive_status(&[]), "stopped");
        assert_eq!(derive_status(&[svc("exited"), svc("exited")]), "stopped");
        assert_eq!(derive_status(&[svc("running"), svc("running")]), "running");
        assert_eq!(derive_status(&[svc("running"), svc("exited")]), "partial");
    }

    #[test]
    fn builds_berth_view() {
        let berth = Berth {
            name: "alpha".into(),
            branch: "alpha".into(),
            worktree: PathBuf::from("/tmp/repo-alpha"),
            compose_project: "berth-alpha".into(),
            compose_file: PathBuf::from("docker-compose.yml"),
            ports: HashMap::from([
                ("WEB_PORT".to_string(), 5001u16),
                ("API_PORT".to_string(), 5002u16),
            ]),
            created_at: 0,
        };

        let view = build_berth_view(&berth, true, &[("BERTH_NAME".into(), "alpha".into())]);

        assert_eq!(view["name"], "alpha");
        assert_eq!(view["status"], "running");
        assert_eq!(view["compose_project"], "berth-alpha");
        // Ports are sorted by env-var name: API_PORT before WEB_PORT.
        assert_eq!(view["ports"][0]["env"], "API_PORT");
        assert_eq!(view["ports"][0]["url"], "http://localhost:5002");
        assert_eq!(view["env"][0]["key"], "BERTH_NAME");
    }
}
