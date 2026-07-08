mod agent;
mod dashboard;
mod docker;
mod doctor;
mod down;
mod env;
mod hooks;
mod ls;
mod ports;
mod state;
mod types;
mod up;
mod validate;
mod worktree;

use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::exit};

/// Give every AI coding agent its own isolated dev environment —
/// isolated worktree, auto-assigned ports, isolated state, one command up/down.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,

    #[arg(short, long, global = true, default_value = ".")]
    dir: PathBuf,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Check that berth.yml and the referenced compose file are valid.
    Validate,

    /// Create a worktree + isolated dev environment with auto-assigned ports.
    Up { name: String },

    /// Tear down a berth's services and volumes and remove its worktree.
    Down {
        name: String,

        /// Tear down even if the branch is unmerged or has uncommitted changes
        /// (discards the work). Use for abandoned agents.
        #[arg(short, long)]
        force: bool,
    },

    /// List every berth: name, branch, status, age, and ports.
    Ls,

    /// Serve a local web dashboard of every berth: live env, ports, status,
    /// and streaming logs, with start/stop/restart/tear-down actions.
    Dashboard {
        /// Port to bind on 127.0.0.1. Defaults to an auto-assigned free port.
        #[arg(long)]
        port: Option<u16>,

        /// Do not open the dashboard in a browser on startup.
        #[arg(long)]
        no_open: bool,
    },

    /// Create a berth, launch an agent inside its worktree, and tear the berth
    /// down (safely) when the agent exits.
    Agent {
        name: String,

        /// The agent command to run, after `--`, e.g. `berth agent x -- claude`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Bring an existing berth's environment up.
    Start { name: String },

    /// Stop an existing berth's environment, keeping its worktree.
    Stop { name: String },

    /// Manage agent-harness hooks (install/uninstall), or run one.
    Hooks {
        #[command(subcommand)]
        action: hooks::HooksCmd,
    },
}

fn main() {
    let args = Args::parse();

    match args.cmd {
        Commands::Validate => match validate::validate(args.dir.clone()) {
            Ok(_) => {
                println!("✓ Config valid");
            }
            Err(errors) => {
                eprintln!("✗ Invalid berth.yml");

                for error in errors {
                    eprintln!("  • {}", error);
                }

                exit(1);
            }
        },

        Commands::Up { name } => {
            if let Err(err) = up::up(&args.dir, &name) {
                eprintln!("✗ {err}");
                exit(1);
            }

            println!("✓ Berth '{name}' is running.");
        }

        Commands::Down { name, force } => {
            if let Err(err) = down::down(&args.dir, &name, force) {
                eprintln!("✗ {err}");
                exit(1);
            }

            println!("✓ Berth '{name}' removed.");
        }

        Commands::Ls => {
            if let Err(err) = ls::list(&args.dir) {
                eprintln!("✗ {err}");
                exit(1);
            }
        }

        Commands::Dashboard { port, no_open } => {
            if let Err(err) = dashboard::serve(&args.dir, port, no_open) {
                eprintln!("✗ {err}");
                exit(1);
            }
        }

        Commands::Agent { name, command } => {
            if let Err(err) = agent::agent(&args.dir, &name, &command) {
                eprintln!("✗ {err}");
                exit(1);
            }
        }

        Commands::Start { name } => {
            if let Err(err) = agent::start(&args.dir, &name) {
                eprintln!("✗ {err}");
                exit(1);
            }
        }

        Commands::Stop { name } => {
            if let Err(err) = agent::stop(&args.dir, &name) {
                eprintln!("✗ {err}");
                exit(1);
            }
        }

        Commands::Hooks { action } => {
            if let Err(err) = hooks::dispatch(action, &args.dir) {
                eprintln!("✗ {err}");
                exit(1);
            }
        }
    }
}
