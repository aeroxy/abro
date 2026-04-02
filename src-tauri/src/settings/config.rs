use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub provider: String,
    pub project_id: String,
    pub location: String,
    pub model: String,
    #[serde(default)]
    pub credentials_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AbroConfig {
    #[serde(default)]
    pub ai: Option<AiProviderConfig>,
}

pub struct ConfigState(pub Arc<Mutex<AbroConfig>>);

impl ConfigState {
    pub fn new(config: AbroConfig) -> Self {
        Self(Arc::new(Mutex::new(config)))
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".abro").join("config.json"))
}

pub fn load_config() -> AbroConfig {
    let Some(path) = config_path() else {
        return AbroConfig::default();
    };
    if !path.exists() {
        return AbroConfig::default();
    }
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => AbroConfig::default(),
    }
}

pub fn persist_config(config: &AbroConfig) -> Result<(), String> {
    let path = config_path().ok_or("cannot resolve home directory")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create ~/.abro: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(config).map_err(|e| format!("serialize error: {e}"))?;
    fs::write(&path, raw).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn get_config(state: tauri::State<'_, ConfigState>) -> Result<AbroConfig, String> {
    let config = state.0.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

#[tauri::command]
pub fn save_config(
    state: tauri::State<'_, ConfigState>,
    config: AbroConfig,
) -> Result<(), String> {
    persist_config(&config)?;
    let mut current = state.0.lock().map_err(|e| e.to_string())?;
    *current = config;
    Ok(())
}
