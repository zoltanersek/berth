mod install;
mod resolve;

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::Value;

use crate::{docker, down, state};
use install::Harness;

#[derive(Subcommand, Debug, Clone)]
pub enum HooksCmd {
    /// Install berth hooks into Claude Code and/or Codex settings.
    /// With neither --claude nor --codex, both are configured.
    Install {
        #[arg(long)]
        claude: bool,
        #[arg(long)]
        codex: bool,
        /// Write to the user's home settings instead of this repo's.
        #[arg(long)]
        global: bool,
    },

    /// Remove berth's hook entries.
    Uninstall {
        #[arg(long)]
        claude: bool,
        #[arg(long)]
        codex: bool,
        #[arg(long)]
        global: bool,
    },

    /// Internal: invoked by an installed hook. Reads the harness event JSON on
    /// stdin and brings the current berth's environment up or tears it down.
    Run { event: String },
}

pub fn dispatch(cmd: HooksCmd, dir: &Path) -> Result<(), String> {
    match cmd {
        HooksCmd::Install {
            claude,
            codex,
            global,
        } => install::install(dir, &selected(claude, codex), global),
        HooksCmd::Uninstall {
            claude,
            codex,
            global,
        } => install::uninstall(dir, &selected(claude, codex), global),
        HooksCmd::Run { event } => run(&event),
    }
}

fn selected(claude: bool, codex: bool) -> Vec<Harness> {
    match (claude, codex) {
        (false, false) => vec![Harness::Claude, Harness::Codex],
        _ => {
            let mut harnesses = Vec::new();
            if claude {
                harnesses.push(Harness::Claude);
            }
            if codex {
                harnesses.push(Harness::Codex);
            }
            harnesses
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start,
    Down,
    Noop,
}

/// What a hook event should do. Kept pure and separate so the policy — in
/// particular, that `clear`/`resume` (the session is being replaced, not
/// finished) must NOT tear down — is exhaustively testable.
fn decide(event: &str, reason: &str) -> Action {
    match event {
        "session-start" => Action::Start,
        "session-end" if reason == "clear" || reason == "resume" => Action::Noop,
        "session-end" => Action::Down,
        _ => Action::Noop,
    }
}

/// A hook must never disrupt the agent, so failures are logged to stderr and we
/// still succeed.
fn run(event: &str) -> Result<(), String> {
    if let Err(e) = try_run(event) {
        eprintln!("berth hooks: {e}");
    }
    Ok(())
}

fn try_run(event: &str) -> Result<(), String> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    let reason = payload.get("reason").and_then(Value::as_str).unwrap_or("");
    let action = decide(event, reason);
    if action == Action::Noop {
        return Ok(());
    }

    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or("no working directory in hook payload")?;

    // Not inside a git worktree / detached HEAD: nothing to manage.
    let Ok(resolved) = resolve::resolve(&cwd) else {
        return Ok(());
    };

    // The agent isn't in a berth-managed worktree: no-op.
    let Some(berth) = state::get(&resolved.root, &resolved.name)? else {
        return Ok(());
    };

    match action {
        Action::Start => {
            let env_file = resolved
                .root
                .join(".berth")
                .join(format!("{}.env", resolved.name));
            docker::up(
                &resolved.root,
                &berth.compose_file,
                &env_file,
                &berth.compose_project,
            )
        }
        Action::Down => down::down(&resolved.root, &resolved.name, false),
        Action::Noop => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_event_always_starts() {
        assert_eq!(decide("session-start", "startup"), Action::Start);
        assert_eq!(decide("session-start", "resume"), Action::Start);
    }

    #[test]
    fn end_event_tears_down_on_a_real_exit() {
        assert_eq!(decide("session-end", "other"), Action::Down);
        assert_eq!(decide("session-end", "logout"), Action::Down);
        assert_eq!(decide("session-end", "prompt_input_exit"), Action::Down);
        assert_eq!(decide("session-end", ""), Action::Down);
    }

    #[test]
    fn end_event_is_noop_when_session_is_only_replaced() {
        assert_eq!(decide("session-end", "clear"), Action::Noop);
        assert_eq!(decide("session-end", "resume"), Action::Noop);
    }

    #[test]
    fn unknown_event_is_noop() {
        assert_eq!(decide("PreToolUse", "anything"), Action::Noop);
    }

    #[test]
    fn selected_defaults_to_both() {
        assert_eq!(selected(false, false).len(), 2);
        assert_eq!(selected(true, false).len(), 1);
        assert_eq!(selected(false, true).len(), 1);
    }
}
