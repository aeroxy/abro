use serde::{Deserialize, Serialize};

/// Terminal Settings - Shell preferences, working directory defaults, agent configuration
/// UI preferences (themes, fonts, etc.)
/// Based on `terminal::terminal_settings::TerminalSettings`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSettings {
    pub shell: String,
    pub working_directory: Option<String>,
    pub agent_enabled: bool,
    pub theme: String,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: "/bin/bash".to_string(),
            working_directory: None,
            agent_enabled: true,
            theme: "dark".to_string(),
        }
    }
}
