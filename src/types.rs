use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub compose: String,
    #[serde(default)]
    pub ports: HashMap<String, PortConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortConfig {
    pub env: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BerthState {
    pub berths: Vec<Berth>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Berth {
    pub name: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub compose_project: String,
    pub compose_file: PathBuf,

    pub ports: HashMap<String, u16>,

    /// Unix timestamp (seconds) when the berth was created, for `ls` age.
    #[serde(default)]
    pub created_at: u64,
}
