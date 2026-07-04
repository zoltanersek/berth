mod docker;
mod doctor;
mod down;
mod env;
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
    }
}
