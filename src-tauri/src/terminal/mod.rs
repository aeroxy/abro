pub mod pty;

use crate::terminal::pty::PtyManager;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{command, AppHandle, State};

pub struct PtyState {
    pub ptys: Arc<Mutex<HashMap<String, Arc<PtyManager>>>>,
}

impl PtyState {
    pub fn new() -> Self {
        Self {
            ptys: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[command]
pub fn spawn_pty(app: AppHandle, state: State<'_, PtyState>) -> Result<String, String> {
    let pty = Arc::new(PtyManager::new());
    if let Err(e) = pty.spawn(app) {
        return Err(e.to_string());
    }
    let id = pty.id.clone();
    state.ptys.lock().unwrap().insert(id.clone(), pty);
    Ok(id)
}

#[command]
pub fn write_pty(state: State<'_, PtyState>, id: String, data: String) -> Result<(), String> {
    if let Some(pty) = state.ptys.lock().unwrap().get(&id) {
        pty.write(data).map_err(|e| e.to_string())
    } else {
        Err("PTY not found".into())
    }
}

#[command]
pub fn resize_pty(
    state: State<'_, PtyState>,
    id: String,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    if let Some(pty) = state.ptys.lock().unwrap().get(&id) {
        pty.resize(rows, cols).map_err(|e| e.to_string())
    } else {
        Err("PTY not found".into())
    }
}

#[command]
pub fn close_pty(state: State<'_, PtyState>, id: String) -> Result<(), String> {
    if let Some(pty) = state.ptys.lock().unwrap().remove(&id) {
        pty.kill();
        Ok(())
    } else {
        Err("PTY not found".into())
    }
}

#[command]
pub fn get_completions(cwd: String, input: String) -> Result<Vec<String>, String> {
    use std::process::Command;

    // Determine the last token. If input ends with whitespace, we are completing a completely new empty token
    let is_new_token = input.ends_with(|c: char| c.is_whitespace());
    let tokens: Vec<&str> = input.split_whitespace().collect();

    let last_token = if is_new_token {
        ""
    } else {
        tokens.last().copied().unwrap_or("")
    };

    // Complete commands if there's only one token (or trailing space after command means we complete file args)
    let comp_type = if tokens.is_empty() || (tokens.len() == 1 && !is_new_token) {
        "-A command -A file"
    } else {
        "-A file"
    };

    let mut resolved_cwd = cwd;
    if resolved_cwd == "~" || resolved_cwd.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            resolved_cwd = resolved_cwd.replacen('~', &home, 1);
        }
    }

    // If current dir fails (e.g. connected to remote SSH and local folder doesn't exist), fallback to /
    let mut cmd = Command::new("bash");
    if std::path::Path::new(&resolved_cwd).exists() {
        cmd.current_dir(&resolved_cwd);
    }

    let output = cmd
        .arg("-c")
        .arg(format!("compgen {} {}", comp_type, last_token))
        .output()
        .map_err(|e| e.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut completions: Vec<String> = stdout.lines().map(|s| s.to_string()).collect();

    // Convert absolute paths back to tilde paths for consistency
    if let Ok(home) = std::env::var("HOME") {
        for completion in completions.iter_mut() {
            if *completion == home {
                *completion = "~".to_string();
            } else if completion.starts_with(&format!("{}/", home)) {
                *completion = format!("~/{}", &completion[home.len() + 1..]);
            }
        }
    }

    // Deduplicate and limit
    completions.sort();
    completions.dedup();
    if completions.len() > 100 {
        completions.truncate(100);
    }

    Ok(completions)
}

#[command]
pub fn get_shell_history() -> Result<Vec<String>, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let mut history_lines: Vec<String> = Vec::new();

    let zsh_path = format!("{}/.zsh_history", home);
    if let Ok(bytes) = std::fs::read(&zsh_path) {
        let content = String::from_utf8_lossy(&bytes);
        let mut current_cmd = String::new();
        for line in content.lines() {
            let mut line_str = line;

            // Extract the actual command if it uses extended history format
            if line.starts_with(": ") {
                if let Some(idx) = line.find(';') {
                    line_str = &line[idx + 1..];
                }
            }

            // Filter out our internal shell integration hooks
            if line_str.contains("__abro_hooks") || line_str.contains("abro_rc_") {
                if let Some(last) = history_lines.last() {
                    if last.contains("ssh ") && last.contains("-t 'echo") {
                        history_lines.pop();
                    }
                }
                current_cmd.clear();
                continue;
            }

            // ZSH escapes multiline commands with a trailing backslash
            if line_str.ends_with('\\') {
                current_cmd.push_str(&line_str[..line_str.len() - 1]);
                current_cmd.push('\n');
            } else {
                current_cmd.push_str(line_str);
                if !current_cmd.trim().is_empty() {
                    history_lines.push(current_cmd.clone());
                }
                current_cmd.clear();
            }
        }
        if !current_cmd.trim().is_empty() {
            history_lines.push(current_cmd);
        }
    } else {
        let bash_path = format!("{}/.bash_history", home);
        if let Ok(content) = std::fs::read_to_string(&bash_path) {
            for line in content.lines() {
                history_lines.push(line.to_string());
            }
        }
    }

    let mut unique = Vec::new();
    // iterate in reverse to get most recent first
    for cmd in history_lines.into_iter().rev() {
        let trimmed = cmd.replace("\\\n", "\n").trim().to_string();
        if !trimmed.is_empty() && !unique.contains(&trimmed) {
            unique.push(trimmed);
        }
        if unique.len() >= 200 {
            break;
        }
    }
    unique.reverse();

    Ok(unique)
}
