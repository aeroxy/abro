use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockFilterModeState {
    #[default]
    All,
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandBlockState {
    pub id: u64,
    pub command: String,
    pub output: String,
    pub collapsed: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockUiState {
    #[serde(default)]
    pub filter_mode: BlockFilterModeState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    #[serde(default)]
    pub recent_tabs: Vec<String>,
    #[serde(default)]
    pub last_provider: Option<String>,
    #[serde(default)]
    pub command_blocks: Vec<CommandBlockState>,
    #[serde(default)]
    pub block_ui: BlockUiState,
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to read state from {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse state from {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write state to {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode state: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<WorkspaceState, PersistenceError> {
        if !self.path.exists() {
            return Ok(WorkspaceState::default());
        }

        let raw = fs::read_to_string(&self.path).map_err(|source| PersistenceError::Read {
            path: self.path.display().to_string(),
            source,
        })?;

        serde_json::from_str(&raw).map_err(|source| PersistenceError::Parse {
            path: self.path.display().to_string(),
            source,
        })
    }

    pub fn save(&self, state: &WorkspaceState) -> Result<(), PersistenceError> {
        let raw = serde_json::to_string_pretty(state)?;
        fs::write(&self.path, raw).map_err(|source| PersistenceError::Write {
            path: self.path.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn state_round_trip() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();

        let path = std::env::temp_dir().join(format!("abro-state-{stamp}.json"));
        let store = StateStore::new(&path);

        let state = WorkspaceState {
            recent_tabs: vec!["main".into(), "logs".into()],
            last_provider: Some("mock".into()),
            command_blocks: vec![
                CommandBlockState {
                    id: 1,
                    command: "ls".into(),
                    output: "Cargo.toml".into(),
                    collapsed: false,
                    exit_code: Some(0),
                },
                CommandBlockState {
                    id: 2,
                    command: "false".into(),
                    output: "".into(),
                    collapsed: true,
                    exit_code: Some(1),
                },
            ],
            block_ui: BlockUiState {
                filter_mode: BlockFilterModeState::Failed,
            },
        };

        store.save(&state).expect("save should succeed");
        let loaded = store.load().expect("load should succeed");

        assert_eq!(loaded, state);
        let _ = fs::remove_file(path);
    }
}
